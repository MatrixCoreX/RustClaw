use rusqlite::Connection;
use serde_json::json;

fn tasks_db() -> Connection {
    let db = Connection::open_in_memory().expect("open tasks db");
    db.execute_batch(
        "CREATE TABLE tasks (
            task_id TEXT PRIMARY KEY,
            status TEXT NOT NULL,
            payload_json TEXT NOT NULL DEFAULT '{}',
            result_json TEXT,
            error_text TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            lease_owner TEXT,
            lease_expires_at INTEGER NOT NULL DEFAULT 0,
            claimed_at INTEGER NOT NULL DEFAULT 0,
            claim_attempt INTEGER NOT NULL DEFAULT 0
        );",
    )
    .expect("create tasks table");
    db
}

fn running_resume_result(checkpoint_id: &str) -> String {
    json!({
        "task_lifecycle": {
            "schema_version": 1,
            "state": "running",
            "checkpoint_id": checkpoint_id,
            "resume_executor": {
                "checkpoint_id": checkpoint_id,
                "executor_state": "executing_planner_resume",
                "resume_directive": "run_next_planner_round"
            },
            "resume_executor_dispatch_claim": {
                "checkpoint_id": checkpoint_id,
                "expires_at": 1
            }
        },
        "task_checkpoint": {
            "schema_version": 1,
            "checkpoint_id": checkpoint_id,
            "boundary_context": {"route_gate_kind": "execute"},
            "observations": [],
            "evidence_refs": [],
            "artifact_refs": [],
            "completed_side_effect_refs": ["skill:completed"],
            "budget": {
                "round": 2,
                "step": 2,
                "llm_calls": 2,
                "tool_calls": 2,
                "elapsed_ms": 1000
            },
            "resume_entrypoint": "next_planner_round"
        }
    })
    .to_string()
}

#[test]
fn stale_worker_lease_preserves_recoverable_running_resume_execution() {
    let db = tasks_db();
    db.execute(
        "INSERT INTO tasks (
            task_id, status, payload_json, result_json, created_at, updated_at,
            lease_owner, lease_expires_at
         ) VALUES ('resume-running', 'running', '{}', ?1, '1', '1', 'old-worker', 1)",
        rusqlite::params![running_resume_result("ckpt-running")],
    )
    .expect("insert recoverable resume");

    let recovered = super::stale_recovery::recover_stale_running_tasks_on_startup(&db, 60)
        .expect("recover stale tasks");

    assert!(recovered.is_empty());
    let (status, error_text): (String, Option<String>) = db
        .query_row(
            "SELECT status, error_text FROM tasks WHERE task_id = 'resume-running'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read recoverable resume");
    assert_eq!(status, "running");
    assert!(error_text.is_none());
}

#[test]
fn startup_adopts_durable_resume_without_waiting_for_old_process_lease() {
    let db = tasks_db();
    let old_lease = crate::now_ts_u64() as i64 + 600;
    db.execute(
        "INSERT INTO tasks (
            task_id, status, payload_json, result_json, created_at, updated_at,
            lease_owner, lease_expires_at, claim_attempt
         ) VALUES ('resume-adopt', 'running', '{}', ?1, '1', '1',
                   'worker:old', ?2, 4)",
        rusqlite::params![running_resume_result("ckpt-adopt"), old_lease],
    )
    .expect("insert durable resume");

    let adopted = super::stale_recovery::adopt_recoverable_resume_executions_on_startup(
        &db,
        "worker:new",
        300,
    )
    .expect("adopt durable resume");
    assert_eq!(adopted, vec!["resume-adopt".to_string()]);

    let (owner, attempt, raw): (String, i64, String) = db
        .query_row(
            "SELECT lease_owner, claim_attempt, result_json FROM tasks WHERE task_id = 'resume-adopt'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("read adopted resume");
    let result: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(owner, "worker:new");
    assert_eq!(attempt, 5);
    assert_eq!(
        result["task_lifecycle"]["resume_claim"]["owner"],
        "worker:new"
    );
    assert_eq!(
        result["task_lifecycle"]["resume_claim"]["recovery_reason"],
        "service_restart"
    );
    assert!(
        result["task_lifecycle"]["resume_executor_dispatch_claim"]["expires_at"]
            .as_i64()
            .is_some_and(|expires_at| expires_at <= crate::now_ts_u64() as i64)
    );
}

#[test]
fn startup_releases_unclaimed_checkpoint_lease_to_normal_recovery() {
    let db = tasks_db();
    let old_lease = crate::now_ts_u64() as i64 + 600;
    let result = json!({
        "task_lifecycle": {
            "schema_version": 1,
            "state": "waiting",
            "checkpoint_id": "ckpt-unclaimed",
            "next_check_after": 1
        },
        "task_checkpoint": {
            "schema_version": 1,
            "checkpoint_id": "ckpt-unclaimed",
            "boundary_context": {},
            "observations": [],
            "evidence_refs": [],
            "artifact_refs": [],
            "completed_side_effect_refs": [],
            "budget": {"round": 0, "step": 1, "llm_calls": 0, "tool_calls": 1, "elapsed_ms": 0},
            "resume_entrypoint": "poll_async_job"
        }
    })
    .to_string();
    db.execute(
        "INSERT INTO tasks (
            task_id, status, payload_json, result_json, created_at, updated_at,
            lease_owner, lease_expires_at, claim_attempt
         ) VALUES ('resume-unclaimed', 'running', '{}', ?1, '1', '1',
                   'worker:old', ?2, 4)",
        rusqlite::params![result, old_lease],
    )
    .unwrap();

    let adopted = super::stale_recovery::adopt_recoverable_resume_executions_on_startup(
        &db,
        "worker:new",
        300,
    )
    .unwrap();
    assert_eq!(adopted, vec!["resume-unclaimed".to_string()]);
    let (owner, lease): (Option<String>, i64) = db
        .query_row(
            "SELECT lease_owner, lease_expires_at FROM tasks WHERE task_id = 'resume-unclaimed'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert!(owner.is_none());
    assert_eq!(lease, 0);
}

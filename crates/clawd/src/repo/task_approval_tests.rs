use rusqlite::{params, Connection};
use serde_json::json;

use super::*;

fn db() -> Connection {
    let db = Connection::open_in_memory().expect("open db");
    db.execute_batch(
        "CREATE TABLE tasks (
            task_id TEXT PRIMARY KEY,
            status TEXT NOT NULL,
            result_json TEXT,
            error_text TEXT,
            updated_at TEXT,
            lease_owner TEXT,
            lease_expires_at INTEGER NOT NULL DEFAULT 0,
            claimed_at INTEGER NOT NULL DEFAULT 0
        );",
    )
    .expect("create tasks");
    db
}

fn binding(arguments_hash: &str) -> ApprovalBinding {
    ApprovalBinding {
        action_fingerprint: "sha256:action".to_string(),
        arguments_hash: arguments_hash.to_string(),
        action_count: 1,
        targets: vec!["write_file".to_string()],
    }
}

fn insert_pending(db: &Connection, expires_at: i64) {
    let result = json!({
        "resume_context": {
            "approval_request": {
                "schema_version": 1,
                "request_id": "approval-1",
                "task_id": "task-1",
                "status": "pending",
                "action_fingerprint": "sha256:action",
                "arguments_hash": "sha256:args",
                "expires_at": expires_at
            }
        }
    });
    db.execute(
        "INSERT INTO tasks (task_id, status, result_json, error_text, updated_at)
         VALUES ('task-1', 'failed', ?1, 'approval required', '0')",
        params![result.to_string()],
    )
    .expect("insert task");
}

#[test]
fn approval_requeues_and_consumes_exact_binding_once() {
    let db = db();
    insert_pending(&db, 500);

    let update = approve_task_approval_request_in_db(&db, "task-1", "approval-1", 100)
        .expect("approve")
        .expect("approval update");
    assert_eq!(update.request_id, "approval-1");
    assert_eq!(update.expires_at, 500);
    db.execute(
        "UPDATE tasks SET status = 'running' WHERE task_id = 'task-1'",
        [],
    )
    .expect("claim task");

    assert_eq!(
        consume_task_approval_grant_in_db(&db, "task-1", &binding("sha256:args"), 110)
            .expect("consume"),
        TaskApprovalConsumeOutcome::Consumed
    );
    assert_eq!(
        consume_task_approval_grant_in_db(&db, "task-1", &binding("sha256:args"), 111)
            .expect("consume replay"),
        TaskApprovalConsumeOutcome::NotApproved
    );
}

#[test]
fn changed_arguments_do_not_consume_approved_grant() {
    let db = db();
    insert_pending(&db, 500);
    approve_task_approval_request_in_db(&db, "task-1", "approval-1", 100)
        .expect("approve")
        .expect("approval update");
    db.execute(
        "UPDATE tasks SET status = 'running' WHERE task_id = 'task-1'",
        [],
    )
    .expect("claim task");

    assert_eq!(
        consume_task_approval_grant_in_db(&db, "task-1", &binding("sha256:changed"), 110)
            .expect("consume changed"),
        TaskApprovalConsumeOutcome::BindingMismatch
    );
}

#[test]
fn expired_request_cannot_be_approved() {
    let db = db();
    insert_pending(&db, 100);

    assert!(
        approve_task_approval_request_in_db(&db, "task-1", "approval-1", 100)
            .expect("approve expired")
            .is_none()
    );
    let (status, result): (String, String) = db
        .query_row(
            "SELECT status, result_json FROM tasks WHERE task_id = 'task-1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("expired task");
    assert_eq!(status, "failed");
    assert_eq!(
        serde_json::from_str::<Value>(&result).expect("result")["resume_context"]
            ["approval_request"]["status"],
        "expired"
    );
}

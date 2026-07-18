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
            claimed_at INTEGER NOT NULL DEFAULT 0,
            user_id INTEGER NOT NULL DEFAULT 42,
            chat_id INTEGER NOT NULL DEFAULT 7,
            user_key TEXT,
            channel TEXT NOT NULL DEFAULT 'web'
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
        scope: None,
    }
}

fn approval_checkpoint() -> Value {
    json!({
        "task_lifecycle": {
            "schema_version": 1,
            "state": "needs_user",
            "resume_reason": "confirmation_required",
            "checkpoint_id": "checkpoint-approval-1"
        },
        "task_checkpoint": {
            "schema_version": 1,
            "checkpoint_id": "checkpoint-approval-1",
            "boundary_context": {},
            "last_successful_round": null,
            "last_successful_step": null,
            "pending_action": null,
            "observations": [],
            "evidence_refs": [],
            "artifact_refs": [],
            "completed_side_effect_refs": [],
            "budget": {
                "round": 0,
                "step": 0,
                "llm_calls": 0,
                "tool_calls": 0,
                "elapsed_ms": 0,
                "llm_elapsed_ms": 0,
                "tool_elapsed_ms": 0
            },
            "resume_entrypoint": "await_user_input"
        }
    })
}

fn insert_pending(db: &Connection, expires_at: i64) {
    let mut result = approval_checkpoint();
    result["resume_context"] = json!({
        "approval_request": {
            "schema_version": 1,
            "request_id": "approval-1",
            "task_id": "task-1",
            "status": "pending",
            "action_fingerprint": "sha256:action",
            "arguments_hash": "sha256:args",
            "expires_at": expires_at
        }
    });
    db.execute(
        "INSERT INTO tasks (
            task_id, status, result_json, error_text, updated_at,
            user_id, chat_id, user_key, channel
         )
         VALUES (
            'task-1', 'running', ?1, NULL, '0',
            42, 7, 'actor-key', 'web'
         )",
        params![result.to_string()],
    )
    .expect("insert task");
}

fn insert_pending_scope(db: &Connection, expires_at: i64) {
    let mut result = approval_checkpoint();
    result["resume_context"] = json!({
        "approval_request": {
                "schema_version": 1,
                "request_id": "approval-scope-1",
                "task_id": "task-1",
                "status": "pending",
                "action_fingerprint": "sha256:action",
                "arguments_hash": "sha256:args",
                "expires_at": expires_at,
                "scope_grant": {
                    "available": true,
                    "scope_kind": "session",
                    "scope_fingerprint": "sha256:scope",
                    "entries": [{
                        "capability": "filesystem.remove_path",
                        "action": "remove_path",
                        "effect": "mutate",
                        "resource_kind": "workspace_path",
                        "resources": ["run/example.txt"]
                    }]
                }
        }
    });
    db.execute(
        "INSERT INTO tasks (
            task_id, status, result_json, error_text, updated_at,
            user_id, chat_id, user_key, channel
         )
         VALUES (
            'task-1', 'running', ?1, NULL, '0',
            42, 7, 'actor-key', 'web'
         )",
        params![result.to_string()],
    )
    .expect("insert scoped task");
}

#[test]
fn approval_resumes_checkpoint_and_consumes_exact_binding_once() {
    let db = db();
    insert_pending(&db, 500);

    let update = decide_task_approval_request_in_db(
        &db,
        "task-1",
        "approval-1",
        ApprovalDecision::ApproveOnce,
        None,
        100,
    )
    .expect("approve")
    .expect("approval update");
    assert_eq!(update.request_id, "approval-1");
    assert_eq!(update.expires_at, 500);
    assert_eq!(update.decision, ApprovalDecision::ApproveOnce);
    let (status, raw_result, lease_owner, lease_expires_at): (String, String, Option<String>, i64) =
        db.query_row(
            "SELECT status, result_json, lease_owner, lease_expires_at
             FROM tasks
             WHERE task_id = 'task-1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("approved checkpoint");
    let result = serde_json::from_str::<Value>(&raw_result).expect("result json");
    assert_eq!(status, "running");
    assert_eq!(result["task_lifecycle"]["state"], "waiting");
    assert_eq!(
        result["task_checkpoint"]["resume_entrypoint"],
        "next_planner_round"
    );
    assert_eq!(lease_owner, None);
    assert_eq!(lease_expires_at, 0);

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
fn failed_task_pending_approval_compatibility_is_rejected() {
    let db = db();
    insert_pending(&db, 500);
    db.execute(
        "UPDATE tasks SET status = 'failed' WHERE task_id = 'task-1'",
        [],
    )
    .expect("force obsolete failed-task shape");

    assert!(decide_task_approval_request_in_db(
        &db,
        "task-1",
        "approval-1",
        ApprovalDecision::ApproveOnce,
        None,
        100,
    )
    .expect("reject obsolete approval shape")
    .is_none());
}

#[test]
fn changed_arguments_do_not_consume_approved_grant() {
    let db = db();
    insert_pending(&db, 500);
    decide_task_approval_request_in_db(
        &db,
        "task-1",
        "approval-1",
        ApprovalDecision::ApproveOnce,
        None,
        100,
    )
    .expect("approve")
    .expect("approval update");
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

    assert!(decide_task_approval_request_in_db(
        &db,
        "task-1",
        "approval-1",
        ApprovalDecision::ApproveOnce,
        None,
        100,
    )
    .expect("approve expired")
    .is_none());
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

#[test]
fn deny_closes_the_exact_request_without_requeueing() {
    let db = db();
    insert_pending(&db, 500);

    let update = decide_task_approval_request_in_db(
        &db,
        "task-1",
        "approval-1",
        ApprovalDecision::Deny,
        None,
        100,
    )
    .expect("deny")
    .expect("denial update");
    assert_eq!(update.decision, ApprovalDecision::Deny);

    let (status, result): (String, String) = db
        .query_row(
            "SELECT status, result_json FROM tasks WHERE task_id = 'task-1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("denied task");
    let result = serde_json::from_str::<Value>(&result).expect("result json");
    assert_eq!(status, "failed");
    assert_eq!(
        result["resume_context"]["approval_request"]["status"],
        "denied"
    );
    assert_eq!(
        result["resume_context"]["approval_request"]["decision"],
        "deny"
    );
    assert_eq!(result["task_lifecycle"]["state"], "failed");
    assert_eq!(
        result["task_lifecycle"]["terminal_reason"],
        "approval_request_denied"
    );

    assert!(decide_task_approval_request_in_db(
        &db,
        "task-1",
        "approval-1",
        ApprovalDecision::ApproveOnce,
        None,
        110,
    )
    .expect("replay decision")
    .is_none());
}

#[test]
fn scoped_approval_requires_exact_actor_and_resumes_with_signed_grant() {
    let db = db();
    insert_pending_scope(&db, 500);

    assert!(decide_task_approval_request_in_db(
        &db,
        "task-1",
        "approval-scope-1",
        ApprovalDecision::AlwaysForScope,
        Some("other-actor"),
        100,
    )
    .expect("reject other actor")
    .is_none());

    let update = decide_task_approval_request_in_db(
        &db,
        "task-1",
        "approval-scope-1",
        ApprovalDecision::AlwaysForScope,
        Some("actor-key"),
        100,
    )
    .expect("create scope grant")
    .expect("scope approval update");
    let grant = update.scope_grant.expect("scope grant");
    assert!(grant.grant_id.starts_with("scope-grant-"));
    assert_eq!(grant.scope_fingerprint, "sha256:scope");

    let (status, raw_result): (String, String) = db
        .query_row(
            "SELECT status, result_json FROM tasks WHERE task_id = 'task-1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("approved task");
    let result = serde_json::from_str::<Value>(&raw_result).expect("result json");
    assert_eq!(status, "running");
    assert_eq!(result["task_lifecycle"]["state"], "waiting");
    assert_eq!(
        result["task_checkpoint"]["resume_entrypoint"],
        "next_planner_round"
    );
    assert_eq!(
        result["resume_context"]["approval_request"]["scope_grant_id"],
        grant.grant_id
    );
    let grant_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM approval_scope_grants WHERE source_task_id = 'task-1'",
            [],
            |row| row.get(0),
        )
        .expect("scope grant count");
    assert_eq!(grant_count, 1);
}

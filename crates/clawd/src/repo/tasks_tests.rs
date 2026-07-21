use serde_json::json;
use uuid::Uuid;

use super::{
    claim_due_paused_checkpoint_task_internal, claim_next_task,
    claim_ready_paused_checkpoint_resume_executor_internal, get_task_query_record,
    list_active_tasks_internal, list_due_paused_checkpoint_tasks_internal,
    list_ready_paused_checkpoint_resume_executors_internal,
    record_paused_checkpoint_resume_executor_state_internal,
    record_paused_checkpoint_resume_work_item_internal, touch_running_task,
    update_task_checkpointed_result, update_task_failure, update_task_failure_with_result,
    update_task_success, update_task_timeout, WorkerTaskWriteRejected,
    WORKER_LEASE_LOST_STATUS_CODE,
};
use crate::child_task_contract::{
    ChildTaskBudget, ChildTaskMergePolicy, ChildTaskPermissionProfile, ChildTaskSpec,
};
use crate::repo::child_tasks::{
    enqueue_child_task_specs, record_child_task_terminal_projection,
    refresh_parent_child_task_merge, ChildTaskParentContext,
};
use crate::repo::{
    cancel_one_task_for_user_chat, cancel_task_by_id, cancel_tasks_for_user_chat,
    get_task_admin_target, pause_task_by_id, resume_task_with_input, TaskResumeControlInput,
};

struct TempDirGuard {
    path: std::path::PathBuf,
}

impl TempDirGuard {
    fn new(prefix: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("rustclaw_{prefix}_{}", Uuid::new_v4().simple()));
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
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

fn set_task_lease(
    state: &crate::AppState,
    task_id: &str,
    lease_owner: &str,
    lease_expires_at: i64,
    claim_attempt: i64,
    claimed_at: i64,
) {
    let db = state.core.db.get().expect("get db");
    db.execute(
        "UPDATE tasks
         SET lease_owner = ?2,
             lease_expires_at = ?3,
             claim_attempt = ?4,
             claimed_at = ?5
         WHERE task_id = ?1",
        rusqlite::params![
            task_id,
            lease_owner,
            lease_expires_at,
            claim_attempt,
            claimed_at
        ],
    )
    .expect("set task lease");
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

fn stored_result_json_optional(state: &crate::AppState, task_id: &str) -> Option<String> {
    let db = state.core.db.get().expect("get db");
    db.query_row(
        "SELECT result_json FROM tasks WHERE task_id = ?1",
        rusqlite::params![task_id],
        |row| row.get(0),
    )
    .expect("select optional result_json")
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

#[test]
fn update_task_failure_records_structured_worker_reason() {
    let state = state_with_tasks_table();
    let task_id = Uuid::new_v4();
    insert_task(&state, &task_id.to_string(), "running", None, 1234);
    set_task_lease(
        &state,
        &task_id.to_string(),
        state.worker.worker_id.as_str(),
        i64::MAX,
        1,
        1234,
    );

    update_task_failure(&state, &task_id.to_string(), 1, "opaque-worker-error")
        .expect("update failure");

    assert_eq!(stored_status(&state, &task_id.to_string()), "failed");
    let result = stored_result_json(&state, &task_id.to_string());
    assert_eq!(result["status_code"], "worker_task_failed");
    assert_eq!(result["reason_code"], "worker_runtime_error");
    assert_eq!(result["message_key"], "clawd.task.worker_failed");
    assert_eq!(result["task_lifecycle"]["state"], "failed");
    assert_eq!(result["task_lifecycle"]["source"], "worker_failure");
    assert_eq!(
        result["task_lifecycle"]["terminal_reason"],
        "worker_runtime_error"
    );
    assert_eq!(
        result["task_lifecycle"]["worker_events"][0]["event_type"],
        "worker_failure"
    );

    let (response, _, _) = get_task_query_record(&state, task_id)
        .expect("query task")
        .expect("task exists");
    let lifecycle = response.lifecycle.expect("lifecycle projection");
    assert_eq!(lifecycle["db_status"], "failed");
    assert_eq!(lifecycle["reason_code"], "worker_runtime_error");
    assert_eq!(lifecycle["terminal_reason"], "worker_runtime_error");
}

#[test]
fn update_task_failure_preserves_structured_terminal_reason() {
    for (error_kind, expected_reason, expected_attribution) in [
        (
            "provider_unavailable",
            "provider_window_exhausted",
            "provider_error",
        ),
        (
            "confirmation_timeout",
            "confirmation_timeout",
            "confirmation_wait",
        ),
        (
            "timeout",
            "tool_timeout_without_async_resume",
            "tool_timeout",
        ),
    ] {
        let state = state_with_tasks_table();
        let task_id = Uuid::new_v4();
        insert_task(&state, &task_id.to_string(), "running", None, 1234);
        set_task_lease(
            &state,
            &task_id.to_string(),
            state.worker.worker_id.as_str(),
            i64::MAX,
            1,
            1234,
        );
        let err = crate::skills::structured_skill_error_from_parts(
            "agent_loop",
            error_kind,
            error_kind,
            None,
            Some(json!({"error_code": error_kind})),
        );

        update_task_failure(&state, &task_id.to_string(), 1, &err).expect("update failure");

        assert_eq!(stored_status(&state, &task_id.to_string()), "failed");
        let result = stored_result_json(&state, &task_id.to_string());
        assert_eq!(result["status_code"], "worker_task_failed");
        assert_eq!(result["reason_code"], expected_reason);
        assert_eq!(result["failure_attribution"], expected_attribution);
        assert_eq!(result["task_lifecycle"]["terminal_reason"], expected_reason);
        assert_eq!(
            result["task_lifecycle"]["failure_attribution"],
            expected_attribution
        );
    }
}

fn task_row_count(state: &crate::AppState, task_id: &str) -> i64 {
    let db = state.core.db.get().expect("get db");
    db.query_row(
        "SELECT COUNT(*) FROM tasks WHERE task_id = ?1",
        rusqlite::params![task_id],
        |row| row.get(0),
    )
    .expect("select task row count")
}

fn sample_repo_child_spec(
    parent_task_id: &str,
    child_task_id: &str,
    required: bool,
) -> ChildTaskSpec {
    ChildTaskSpec {
        parent_task_id: parent_task_id.to_string(),
        child_task_id: child_task_id.to_string(),
        role: if required { "explorer" } else { "verifier" }.to_string(),
        scope: json!({
            "objective": format!("machine_child_objective:{child_task_id}"),
            "scope_ref": "workspace:current"
        }),
        permission_profile: ChildTaskPermissionProfile::ReadOnly,
        required,
        budget: ChildTaskBudget::readonly_default(),
        result_contract: json!({
            "kind": "structured_findings",
            "required_keys": ["finding_refs", "evidence_refs"]
        }),
        merge_policy: ChildTaskMergePolicy::StructuredFindings,
    }
}

#[test]
fn update_task_success_can_replace_async_poll_projection_without_visible_reply() {
    let state = state_with_tasks_table();
    let initial = json!({
        "schema_version": 1,
        "source": "local_process_async_job",
        "output": "RUSTCLAW_ASYNC_SMOKE\n",
        "task_lifecycle": {
            "schema_version": 1,
            "state": "succeeded",
            "terminal_executor_action": "poll_async_job",
            "terminal_executor_result_status": "async_poll_completed",
            "resume_executor_result_projection": {
                "final_result_json": {
                    "output": "RUSTCLAW_ASYNC_SMOKE\n"
                }
            }
        }
    });
    insert_task(
        &state,
        "async-visible-replace",
        "succeeded",
        Some(&initial),
        1,
    );
    set_task_lease(
        &state,
        "async-visible-replace",
        state.worker.worker_id.as_str(),
        i64::MAX,
        1,
        1,
    );

    super::update_task_success(
        &state,
        "async-visible-replace",
        1,
        &json!({
            "text": "checkpoint_id=ckpt",
            "messages": ["checkpoint_id=ckpt"]
        })
        .to_string(),
    )
    .expect("update success");

    let result = stored_result_json(&state, "async-visible-replace");
    assert_eq!(result["messages"][0], "checkpoint_id=ckpt");
    assert_eq!(result.get("output"), None);
}

#[test]
fn claim_next_task_records_worker_lease_fields() {
    let state = state_with_tasks_table();
    let task_id = Uuid::new_v4().to_string();
    insert_task(&state, &task_id, "queued", None, crate::now_ts_u64() as i64);

    let claimed = claim_next_task(&state).expect("claim task").expect("task");
    assert_eq!(claimed.task_id, task_id);

    let db = state.core.db.get().expect("get db");
    let (status, lease_owner, lease_expires_at, claim_attempt, claimed_at): (
        String,
        String,
        i64,
        i64,
        i64,
    ) = db
        .query_row(
            "SELECT status, lease_owner, lease_expires_at, claim_attempt, claimed_at
             FROM tasks
             WHERE task_id = ?1",
            rusqlite::params![task_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .expect("select lease fields");

    assert_eq!(status, "running");
    assert_eq!(lease_owner, state.worker.worker_id);
    assert_eq!(claim_attempt, 1);
    assert!(claimed_at > 0);
    assert!(lease_expires_at > claimed_at);
}

#[test]
fn cancel_parent_task_cancels_structured_child_tasks_only() {
    let state = state_with_tasks_table();
    insert_task(
        &state,
        "task-parent",
        "running",
        Some(&json!({
            "child_task_ids": ["task-child-running", "task-child-done"],
            "child_results": [
                {
                    "child_task_id": "task-child-queued",
                    "status": "queued"
                },
                {
                    "child_task_id": "invalid child prose",
                    "status": "queued"
                }
            ],
            "text": "task-child-prose-only"
        })),
        1,
    );
    insert_task(&state, "task-child-running", "running", Some(&json!({})), 1);
    insert_task(&state, "task-child-queued", "queued", None, 1);
    insert_task(&state, "task-child-done", "succeeded", Some(&json!({})), 1);
    insert_task(&state, "task-child-prose-only", "queued", None, 1);

    let affected = cancel_task_by_id(&state, "task-parent").expect("cancel parent");

    assert_eq!(affected, 3);
    assert_eq!(stored_status(&state, "task-parent"), "canceled");
    assert_eq!(stored_status(&state, "task-child-running"), "canceled");
    assert_eq!(stored_status(&state, "task-child-queued"), "canceled");
    assert_eq!(stored_status(&state, "task-child-done"), "succeeded");
    assert_eq!(stored_status(&state, "task-child-prose-only"), "queued");

    let parent = stored_result_json(&state, "task-parent");
    let child = stored_result_json(&state, "task-child-running");
    assert_eq!(parent["terminal_reason"], "user_cancelled");
    assert_eq!(child["terminal_reason"], "parent_cancelled");
    assert_eq!(child["message_key"], "clawd.task.parent_cancelled");
    assert_eq!(
        child["task_lifecycle"]["terminal_reason"],
        "parent_cancelled"
    );
}

#[test]
fn enqueue_child_specs_creates_independent_child_tasks_and_parent_cancel_fanout() {
    let state = state_with_tasks_table();
    insert_task(
        &state,
        "task-parent-enqueue",
        "running",
        Some(&json!({})),
        1,
    );
    let parent = ChildTaskParentContext {
        parent_task_id: "task-parent-enqueue".to_string(),
        user_id: 42,
        chat_id: 7,
        user_key: Some("test-key".to_string()),
        channel: "ui".to_string(),
        external_user_id: Some("ui-user".to_string()),
        external_chat_id: Some("ui-chat".to_string()),
    };
    let specs = vec![
        sample_repo_child_spec("task-parent-enqueue", "task-child-enqueue-1", true),
        sample_repo_child_spec("task-parent-enqueue", "task-child-enqueue-2", false),
    ];

    let summary =
        enqueue_child_task_specs(&state, &parent, &specs, 2, 1).expect("enqueue child specs");

    assert_eq!(summary["status"], "scheduled");
    assert_eq!(summary["queued_child_count"], 2);
    assert_eq!(stored_status(&state, "task-child-enqueue-1"), "queued");
    assert_eq!(stored_status(&state, "task-child-enqueue-2"), "queued");
    let parent_result = stored_result_json(&state, "task-parent-enqueue");
    assert_eq!(parent_result["child_task_ids"][0], "task-child-enqueue-1");
    assert_eq!(parent_result["child_task_ids"][1], "task-child-enqueue-2");
    assert_eq!(
        parent_result["child_task_enqueue"]["scheduler"]["decision"],
        "scheduled"
    );

    let claimed = claim_next_task(&state)
        .expect("claim child")
        .expect("queued child");
    assert_eq!(claimed.task_id, "task-child-enqueue-1");
    assert_eq!(stored_status(&state, "task-child-enqueue-1"), "running");
    let payload: serde_json::Value =
        serde_json::from_str(&claimed.payload_json).expect("parse child payload");
    assert_eq!(payload["task_role"], "subagent_child");
    assert_eq!(payload["parent_task_id"], "task-parent-enqueue");
    assert_eq!(
        payload["child_task_contract"]["permission_profile"],
        "read_only"
    );
    assert_eq!(
        payload["text"],
        "machine_child_objective:task-child-enqueue-1"
    );
    let child_result = stored_result_json(&state, "task-child-enqueue-1");
    assert_eq!(child_result["source"], "child_task_enqueue");
    assert_eq!(
        child_result["task_lifecycle"]["state_source"],
        "child_task_enqueue"
    );

    let affected = cancel_task_by_id(&state, "task-parent-enqueue")
        .expect("cancel parent with enqueued children");

    assert_eq!(affected, 3);
    assert_eq!(stored_status(&state, "task-parent-enqueue"), "canceled");
    assert_eq!(stored_status(&state, "task-child-enqueue-1"), "canceled");
    assert_eq!(stored_status(&state, "task-child-enqueue-2"), "canceled");
    let cancelled_child = stored_result_json(&state, "task-child-enqueue-2");
    assert_eq!(cancelled_child["terminal_reason"], "parent_cancelled");
    assert_eq!(
        cancelled_child["task_lifecycle"]["terminal_reason"],
        "parent_cancelled"
    );
}

#[test]
fn enqueue_child_specs_bounds_by_role_and_permission_profile() {
    let state = state_with_tasks_table();
    insert_task(
        &state,
        "task-parent-role-profile",
        "running",
        Some(&json!({})),
        1,
    );
    let parent = ChildTaskParentContext {
        parent_task_id: "task-parent-role-profile".to_string(),
        user_id: 42,
        chat_id: 7,
        user_key: Some("test-key".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
    };
    let mut first_write =
        sample_repo_child_spec("task-parent-role-profile", "task-child-role-write-1", true);
    first_write.role = "workspace_writer".to_string();
    first_write.permission_profile = ChildTaskPermissionProfile::LocalCurrentWorkspace;
    let mut second_write =
        sample_repo_child_spec("task-parent-role-profile", "task-child-role-write-2", false);
    second_write.role = "workspace_writer".to_string();
    second_write.permission_profile = ChildTaskPermissionProfile::LocalCurrentWorkspace;
    let read_only =
        sample_repo_child_spec("task-parent-role-profile", "task-child-role-read-1", false);
    let specs = vec![first_write, second_write, read_only];

    let summary =
        enqueue_child_task_specs(&state, &parent, &specs, 3, 1).expect("enqueue child specs");

    assert_eq!(summary["status"], "scheduled");
    assert_eq!(summary["queued_child_count"], 2);
    assert_eq!(
        summary["scheduler"]["decision"],
        "role_profile_bounded_partial"
    );
    assert_eq!(
        summary["scheduler"]["skipped_child_tasks"][0]["child_task_id"],
        "task-child-role-write-2"
    );
    assert_eq!(
        summary["scheduler"]["skipped_child_tasks"][0]["reason_code"],
        "child_role_profile_capacity_exceeded"
    );
    assert_eq!(stored_status(&state, "task-child-role-write-1"), "queued");
    assert_eq!(stored_status(&state, "task-child-role-read-1"), "queued");
    assert_eq!(task_row_count(&state, "task-child-role-write-2"), 0);
    let parent_result = stored_result_json(&state, "task-parent-role-profile");
    assert_eq!(
        parent_result["child_task_ids"][0],
        "task-child-role-write-1"
    );
    assert_eq!(parent_result["child_task_ids"][1], "task-child-role-read-1");
    assert_eq!(
        parent_result["child_task_enqueue"]["scheduler"]["role_profile_boundaries_applied"],
        true
    );
}

#[test]
fn child_terminal_projection_uses_machine_contract_not_visible_text() {
    let state = state_with_tasks_table();
    let spec = sample_repo_child_spec("task-parent-projection", "task-child-projection", true);
    let payload = json!({
        "text": "visible child objective",
        "task_role": "subagent_child",
        "parent_task_id": spec.parent_task_id,
        "child_task_id": spec.child_task_id,
        "child_task_contract": spec.to_json()
    });
    let db = state.core.db.get().expect("get db");
    db.execute(
        "INSERT INTO tasks (
            task_id, user_id, chat_id, user_key, channel, kind, payload_json,
            status, result_json, error_text, created_at, updated_at
        )
        VALUES (?1, 42, 7, 'test-key', 'ui', 'ask', ?2, 'succeeded', ?3, NULL, '1', '1')",
        rusqlite::params![
            spec.child_task_id,
            payload.to_string(),
            json!({
                "text": "visible prose must not become merge data",
                "task_journal": {
                    "summary": {
                        "final_status": "success"
                    }
                }
            })
            .to_string()
        ],
    )
    .expect("insert child task");
    drop(db);

    assert!(
        record_child_task_terminal_projection(&state, "task-child-projection", &payload)
            .expect("record child projection")
    );

    let result = stored_result_json(&state, "task-child-projection");
    assert_eq!(result["child_task_result"]["status"], "succeeded");
    assert_eq!(result["child_task_result"]["required"], true);
    assert!(result["child_task_result"].get("text").is_none());
    assert!(result["child_task_result"].get("error_text").is_none());
    assert_eq!(
        result["child_task_result"]["evidence_refs"][0],
        "task:task-child-projection:result_json"
    );
    assert_eq!(
        result["child_task_result"]["finding_refs"][0],
        "child_task:task-child-projection:structured_result"
    );
    assert_eq!(
        result["task_lifecycle"]["state_source"],
        "child_task_terminal_projection"
    );
}

#[test]
fn parent_child_merge_continues_from_structured_findings_only() {
    let state = state_with_tasks_table();
    insert_task(
        &state,
        "task-parent-merge",
        "running",
        Some(&json!({
            "child_task_ids": ["task-child-merge-required", "task-child-merge-optional"],
            "text": "visible parent prose must not affect child merge"
        })),
        1,
    );
    for (child_task_id, required, status) in [
        ("task-child-merge-required", true, "succeeded"),
        ("task-child-merge-optional", false, "failed"),
    ] {
        let spec = sample_repo_child_spec("task-parent-merge", child_task_id, required);
        let payload = json!({
            "text": "visible child objective",
            "task_role": "subagent_child",
            "parent_task_id": spec.parent_task_id,
            "child_task_id": spec.child_task_id,
            "child_task_contract": spec.to_json()
        });
        let db = state.core.db.get().expect("get db");
        db.execute(
            "INSERT INTO tasks (
                task_id, user_id, chat_id, user_key, channel, kind, payload_json,
                status, result_json, error_text, created_at, updated_at
            )
            VALUES (?1, 42, 7, 'test-key', 'ui', 'ask', ?2, ?3, ?4, NULL, '1', '1')",
            rusqlite::params![
                child_task_id,
                payload.to_string(),
                status,
                json!({
                    "text": "visible child prose must not become parent merge data",
                    "error_text": "visible child error must not become parent merge data"
                })
                .to_string()
            ],
        )
        .expect("insert child task");
        drop(db);
        assert!(
            record_child_task_terminal_projection(&state, child_task_id, &payload)
                .expect("record child projection")
        );
    }

    let parent = stored_result_json(&state, "task-parent-merge");
    let merge = &parent["child_task_merge"];
    assert_eq!(merge["source"], "child_task_parent_merge");
    assert_eq!(merge["parent_continuation"]["status"], "ready");
    assert_eq!(merge["parent_continuation"]["can_continue"], true);
    assert_eq!(merge["merge"]["status"], "partial");
    assert_eq!(merge["merge"]["required_failed_count"], 0);
    assert_eq!(merge["merge"]["optional_failed_count"], 1);
    assert_eq!(
        merge["merge"]["finding_refs"][0],
        "child_task:task-child-merge-required:structured_result"
    );
    assert!(merge.to_string().find("visible child prose").is_none());
    assert!(merge.to_string().find("visible child error").is_none());
}

#[test]
fn parent_child_merge_waits_for_pending_and_blocks_required_failure() {
    let state = state_with_tasks_table();
    insert_task(
        &state,
        "task-parent-blocked",
        "running",
        Some(&json!({
            "child_task_ids": ["task-child-blocked-required", "task-child-blocked-pending"]
        })),
        1,
    );
    insert_task(&state, "task-child-blocked-pending", "running", None, 1);
    let waiting = refresh_parent_child_task_merge(&state, "task-parent-blocked")
        .expect("refresh parent merge")
        .expect("waiting merge");
    assert_eq!(waiting["parent_continuation"]["status"], "waiting");
    assert_eq!(waiting["pending_child_count"], 1);

    let spec = sample_repo_child_spec("task-parent-blocked", "task-child-blocked-required", true);
    let payload = json!({
        "text": "visible child objective",
        "task_role": "subagent_child",
        "parent_task_id": spec.parent_task_id,
        "child_task_id": spec.child_task_id,
        "child_task_contract": spec.to_json()
    });
    let db = state.core.db.get().expect("get db");
    db.execute(
        "INSERT INTO tasks (
            task_id, user_id, chat_id, user_key, channel, kind, payload_json,
            status, result_json, error_text, created_at, updated_at
        )
        VALUES (?1, 42, 7, 'test-key', 'ui', 'ask', ?2, 'failed', ?3, NULL, '1', '1')",
        rusqlite::params![
            spec.child_task_id,
            payload.to_string(),
            json!({"status_code": "machine_failure"}).to_string()
        ],
    )
    .expect("insert required child");
    drop(db);
    assert!(
        record_child_task_terminal_projection(&state, "task-child-blocked-required", &payload)
            .expect("record child projection")
    );

    let db = state.core.db.get().expect("get db");
    db.execute(
        "UPDATE tasks SET status = 'succeeded', result_json = ?2 WHERE task_id = ?1",
        rusqlite::params![
            "task-child-blocked-pending",
            json!({
                "child_task_result": {
                    "schema_version": 1,
                    "parent_task_id": "task-parent-blocked",
                    "child_task_id": "task-child-blocked-pending",
                    "role": "verifier",
                    "required": false,
                    "status": "succeeded",
                    "evidence_refs": ["task:task-child-blocked-pending:result_json"],
                    "finding_refs": ["child_task:task-child-blocked-pending:structured_result"]
                }
            })
            .to_string()
        ],
    )
    .expect("complete optional child");
    drop(db);
    let blocked = refresh_parent_child_task_merge(&state, "task-parent-blocked")
        .expect("refresh parent merge")
        .expect("blocked merge");
    assert_eq!(blocked["parent_continuation"]["status"], "blocked");
    assert_eq!(
        blocked["parent_continuation"]["reason_code"],
        "required_child_failed"
    );
    assert_eq!(blocked["merge"]["required_failed_count"], 1);
    assert_eq!(blocked["merge"]["parent_can_continue"], false);
}

#[test]
fn claim_next_task_atomic_claim_prevents_duplicate_execution() {
    let state = state_with_tasks_table();
    let mut second_worker = state.clone();
    second_worker.worker.worker_id = "worker:test-second".to_string();
    let task_id = Uuid::new_v4().to_string();
    insert_task(&state, &task_id, "queued", None, crate::now_ts_u64() as i64);

    let first = claim_next_task(&state)
        .expect("first claim")
        .expect("first worker should claim queued task");
    let second = claim_next_task(&second_worker).expect("second claim");

    assert_eq!(first.task_id, task_id);
    assert_eq!(first.claim_attempt, 1);
    assert!(second.is_none());

    let db = state.core.db.get().expect("get db");
    let (status, lease_owner, claim_attempt): (String, String, i64) = db
        .query_row(
            "SELECT status, lease_owner, claim_attempt
             FROM tasks
             WHERE task_id = ?1",
            rusqlite::params![task_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("select claimed task");

    assert_eq!(status, "running");
    assert_eq!(lease_owner, state.worker.worker_id);
    assert_eq!(claim_attempt, 1);
}

#[test]
fn touch_running_task_renews_worker_lease_fields() {
    let state = state_with_tasks_table();
    let task_id = Uuid::new_v4().to_string();
    insert_task(
        &state,
        &task_id,
        "running",
        None,
        crate::now_ts_u64() as i64,
    );
    set_task_lease(
        &state,
        &task_id,
        state.worker.worker_id.as_str(),
        i64::MAX,
        1,
        crate::now_ts_u64() as i64,
    );

    assert!(touch_running_task(&state, &task_id, 1).expect("touch running task"));

    let db = state.core.db.get().expect("get db");
    let (lease_owner, lease_expires_at): (String, i64) = db
        .query_row(
            "SELECT lease_owner, lease_expires_at FROM tasks WHERE task_id = ?1",
            rusqlite::params![task_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("select lease fields");

    assert_eq!(lease_owner, state.worker.worker_id);
    assert!(lease_expires_at > crate::now_ts_u64() as i64);
}

fn assert_worker_lease_lost(error: anyhow::Error, operation: &str) {
    let rejection = error
        .downcast_ref::<WorkerTaskWriteRejected>()
        .expect("typed worker write rejection");
    assert_eq!(rejection.status_code, WORKER_LEASE_LOST_STATUS_CODE);
    assert_eq!(rejection.operation, operation);
}

#[test]
fn stale_worker_cannot_renew_or_finalize_after_owner_takeover() {
    let stale_worker = state_with_tasks_table();
    let mut current_worker = stale_worker.clone();
    current_worker.worker.worker_id = "worker:takeover".to_string();
    let now = crate::now_ts_u64() as i64;
    for task_id in [
        "lease-heartbeat",
        "lease-progress",
        "lease-success",
        "lease-failure",
        "lease-failure-result",
        "lease-checkpoint",
        "lease-timeout",
    ] {
        insert_task(&stale_worker, task_id, "running", None, now);
        set_task_lease(
            &stale_worker,
            task_id,
            current_worker.worker.worker_id.as_str(),
            now + 600,
            2,
            now,
        );
    }

    assert!(!touch_running_task(&stale_worker, "lease-heartbeat", 1).expect("stale heartbeat"));
    assert_worker_lease_lost(
        super::update_task_progress_result(
            &stale_worker,
            "lease-progress",
            1,
            r#"{"progress_messages":["stale"]}"#,
        )
        .expect_err("stale progress must be fenced"),
        "update_task_progress_result",
    );
    assert_worker_lease_lost(
        update_task_success(&stale_worker, "lease-success", 1, r#"{"status":"ok"}"#)
            .expect_err("stale success must be fenced"),
        "update_task_success",
    );
    assert_worker_lease_lost(
        update_task_failure(&stale_worker, "lease-failure", 1, "worker_runtime_error")
            .expect_err("stale failure must be fenced"),
        "update_task_failure",
    );
    assert_worker_lease_lost(
        update_task_failure_with_result(
            &stale_worker,
            "lease-failure-result",
            1,
            r#"{"status":"error"}"#,
            "worker_runtime_error",
        )
        .expect_err("stale failure result must be fenced"),
        "update_task_failure_with_result",
    );
    assert_worker_lease_lost(
        update_task_checkpointed_result(
            &stale_worker,
            "lease-checkpoint",
            1,
            r#"{"task_lifecycle":{"state":"waiting"}}"#,
        )
        .expect_err("stale checkpoint must be fenced"),
        "update_task_checkpointed_result",
    );
    assert_worker_lease_lost(
        update_task_timeout(&stale_worker, "lease-timeout", 1, "worker_task_timeout")
            .expect_err("stale timeout must be fenced"),
        "update_task_timeout",
    );

    let db = stale_worker.core.db.get().expect("get db");
    for task_id in [
        "lease-heartbeat",
        "lease-progress",
        "lease-success",
        "lease-failure",
        "lease-failure-result",
        "lease-checkpoint",
        "lease-timeout",
    ] {
        let (status, owner): (String, String) = db
            .query_row(
                "SELECT status, lease_owner FROM tasks WHERE task_id = ?1",
                rusqlite::params![task_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("load fenced task");
        assert_eq!(status, "running");
        assert_eq!(owner, current_worker.worker.worker_id);
    }
}

#[test]
fn stale_claim_generation_is_fenced_even_when_worker_id_is_reused() {
    let state = state_with_tasks_table();
    let now = crate::now_ts_u64() as i64;
    for task_id in [
        "generation-heartbeat",
        "generation-progress",
        "generation-success",
    ] {
        insert_task(&state, task_id, "running", None, now);
        set_task_lease(
            &state,
            task_id,
            state.worker.worker_id.as_str(),
            now + 600,
            2,
            now,
        );
    }

    assert!(
        !touch_running_task(&state, "generation-heartbeat", 1).expect("stale generation heartbeat")
    );
    let progress_error = super::update_task_progress_result(
        &state,
        "generation-progress",
        1,
        r#"{"progress_messages":["stale generation"]}"#,
    )
    .expect_err("stale generation progress must be fenced");
    assert_worker_lease_lost(progress_error, "update_task_progress_result");

    let completion_error =
        update_task_success(&state, "generation-success", 1, r#"{"status":"ok"}"#)
            .expect_err("stale generation completion must be fenced");
    let rejection = completion_error
        .downcast_ref::<WorkerTaskWriteRejected>()
        .expect("typed generation rejection");
    assert_eq!(rejection.status_code, WORKER_LEASE_LOST_STATUS_CODE);
    assert_eq!(rejection.expected_claim_attempt, 1);
    assert_eq!(rejection.active_claim_attempt, Some(2));
    assert_eq!(
        rejection.lease_owner.as_deref(),
        Some(state.worker.worker_id.as_str())
    );

    for task_id in [
        "generation-heartbeat",
        "generation-progress",
        "generation-success",
    ] {
        assert_eq!(stored_status(&state, task_id), "running");
        assert_eq!(stored_result_json_optional(&state, task_id), None);
    }
}

#[test]
fn current_owner_can_complete_only_once() {
    let state = state_with_tasks_table();
    let task_id = "lease-complete-once";
    let now = crate::now_ts_u64() as i64;
    insert_task(&state, task_id, "running", None, now);
    set_task_lease(
        &state,
        task_id,
        state.worker.worker_id.as_str(),
        now + 600,
        1,
        now,
    );

    update_task_success(&state, task_id, 1, r#"{"sequence":1}"#).expect("first completion");
    let second = update_task_success(&state, task_id, 1, r#"{"sequence":2}"#)
        .expect_err("second completion must be rejected");
    let rejection = second
        .downcast_ref::<WorkerTaskWriteRejected>()
        .expect("typed state conflict");
    assert_eq!(rejection.status_code, "worker_task_state_conflict");
    assert_eq!(stored_result_json(&state, task_id)["sequence"], 1);
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
        },
        "task_checkpoint": checkpoint_json("ckpt-query", vec!["write_file:tmp/a.txt"])
    });
    insert_task(
        &state,
        &task_id.to_string(),
        "running",
        Some(&progress),
        1234,
    );
    set_task_lease(
        &state,
        &task_id.to_string(),
        "worker:test-query",
        1300,
        2,
        1200,
    );

    let (response, _, _) = get_task_query_record(&state, task_id)
        .expect("query task")
        .expect("task exists");

    assert!(matches!(
        response.status,
        claw_core::types::TaskStatus::Running
    ));
    assert_eq!(
        response.execution_state,
        Some(claw_core::types::TaskExecutionState::Waiting)
    );
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
    assert_eq!(lifecycle["execution_state"], "waiting");
    assert_eq!(lifecycle["db_status"], "running");
    assert_eq!(lifecycle["resume_reason"], "provider_gap_retry_window");
    assert_eq!(lifecycle["reason_code"], "provider_gap_retry_window");
    assert_eq!(lifecycle["checkpoint_id"], "ckpt-query");
    assert_eq!(lifecycle["last_heartbeat_ts"], 1234);
    assert_eq!(lifecycle["heartbeat_at"], 1234);
    assert_eq!(lifecycle["lease_owner"], "worker:test-query");
    assert_eq!(lifecycle["lease_expires_at"], 1300);
    assert_eq!(lifecycle["claim_attempt"], 2);
    assert_eq!(lifecycle["attempt_id"], 2);
    assert_eq!(lifecycle["claimed_at"], 1200);
    assert_eq!(lifecycle["resume_directive"], "run_next_planner_round");
    let directive_payload = &lifecycle["resume_directive_payload"];
    assert_eq!(directive_payload["checkpoint_id"], "ckpt-query");
    assert_eq!(directive_payload["completed_side_effect_count"], 1);
    assert_eq!(directive_payload["requires_idempotency_guard"], true);
}

#[test]
fn list_active_tasks_exposes_lifecycle_projection() {
    let state = state_with_tasks_table();
    let mut checkpoint = checkpoint_json("ckpt-active", vec![]);
    checkpoint["resume_entrypoint"] = json!("poll_async_job");
    checkpoint["pending_async_job"] = json!({
        "job_id": "job-17",
        "status": "running",
        "poll_after_seconds": 9,
        "expires_at": 4102444800_i64,
        "cancel_ref": "cancel:job-17",
        "message_key": "tool.job.running"
    });
    let progress = json!({
        "task_lifecycle": {
            "state": "background",
            "resume_reason": "async_job_poll",
            "next_check_after": 1781800400,
            "checkpoint_id": "ckpt-active",
            "pending_job_ref": "job-17"
        },
        "task_checkpoint": checkpoint
    });
    insert_task(&state, "task-active-1", "running", Some(&progress), 2222);
    set_task_lease(&state, "task-active-1", "worker:test-active", 2400, 3, 2200);

    let tasks = list_active_tasks_internal(&state, 42, 7, None).expect("list active tasks");

    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_id, "task-active-1");
    assert_eq!(tasks[0].status, "running");
    assert_eq!(tasks[0].execution_state, "background");
    let lifecycle = tasks[0].lifecycle.as_ref().expect("lifecycle projection");
    assert_eq!(lifecycle["state"], "background");
    assert_eq!(lifecycle["execution_state"], "background");
    assert_eq!(lifecycle["resume_reason"], "async_job_poll");
    assert_eq!(lifecycle["reason_code"], "async_job_poll");
    assert_eq!(lifecycle["pending_job_ref"], "job-17");
    assert_eq!(lifecycle["poll_ref"], "job-17");
    assert_eq!(lifecycle["resume_directive"], "poll_async_job");
    let directive_payload = &lifecycle["resume_directive_payload"];
    assert_eq!(directive_payload["checkpoint_id"], "ckpt-active");
    assert_eq!(directive_payload["job_id"], "job-17");
    assert_eq!(directive_payload["cancel_ref"], "cancel:job-17");
    assert_eq!(lifecycle["last_heartbeat_ts"], 2222);
    assert_eq!(lifecycle["heartbeat_at"], 2222);
    assert_eq!(lifecycle["lease_owner"], "worker:test-active");
    assert_eq!(lifecycle["lease_expires_at"], 2400);
    assert_eq!(lifecycle["claim_attempt"], 3);
    assert_eq!(lifecycle["attempt_id"], 3);
    assert_eq!(lifecycle["claimed_at"], 2200);
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
fn cancel_task_by_id_runs_local_process_cancel_adapter() {
    let state = state_with_tasks_table();
    let task_id = Uuid::new_v4().to_string();
    let dir = TempDirGuard::new("task_admin_local_process_cancel");
    std::fs::write(dir.path.join("pid"), "not-a-pid\n").expect("write pid");
    let cancel_ref = format!("local_process:{}", dir.path.display());
    let result_json = json!({
        "task_checkpoint": {
            "pending_async_job": {
                "job_id": "local_process:test-cancel",
                "status": "running",
                "poll_after_seconds": 10,
                "expires_at": 9999,
                "cancel_ref": cancel_ref,
                "message_key": "clawd.task.async_job_pending"
            }
        }
    });
    insert_task(&state, &task_id, "running", Some(&result_json), 1234);

    let canceled = cancel_task_by_id(&state, &task_id).expect("cancel task");

    assert_eq!(canceled, 1);
    assert!(dir.path.join("cancel_requested_at").exists());
    assert_eq!(
        std::fs::read_to_string(dir.path.join("cancel_signal"))
            .expect("read cancel signal")
            .trim(),
        "TERM"
    );
    let result = stored_result_json(&state, &task_id);
    assert_eq!(
        result["cancel_adapter_result"]["adapter_kind"],
        "local_process_poll"
    );
    assert_eq!(result["cancel_adapter_result"]["status"], "failed");
    assert_eq!(
        result["cancel_adapter_result"]["error_code"],
        "local_process_cancel_pid_invalid"
    );
    assert_eq!(
        result["task_lifecycle"]["cancel_adapter_kind"],
        "local_process_poll"
    );
    assert_eq!(
        result["task_lifecycle"]["cancel_adapter_result"]["adapter_kind"],
        "local_process_poll"
    );
    assert!(result["cancel_adapter_result"].get("text").is_none());
    assert!(result["cancel_adapter_result"].get("error_text").is_none());
}

#[cfg(unix)]
#[test]
fn cancel_task_by_id_terminates_local_process_group() {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    use std::time::Duration;

    let state = state_with_tasks_table();
    let task_id = Uuid::new_v4().to_string();
    let dir = TempDirGuard::new("task_admin_local_process_group_cancel");
    let mut child = Command::new("bash");
    child
        .arg("-lc")
        .arg("sleep 60")
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = child.spawn().expect("spawn process group child");
    let pid = child.id();
    std::fs::write(dir.path.join("pid"), pid.to_string()).expect("write pid");
    let cancel_ref = format!("local_process:{}", dir.path.display());
    let result_json = json!({
        "task_checkpoint": {
            "pending_async_job": {
                "job_id": "local_process:test-process-group-cancel",
                "status": "running",
                "poll_after_seconds": 10,
                "expires_at": 9999,
                "cancel_ref": cancel_ref,
                "message_key": "clawd.task.async_job_pending"
            }
        }
    });
    insert_task(&state, &task_id, "running", Some(&result_json), 1234);

    let canceled = cancel_task_by_id(&state, &task_id).expect("cancel task");

    assert_eq!(canceled, 1);
    let mut exited = false;
    for _ in 0..30 {
        if child.try_wait().expect("try wait").is_some() {
            exited = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    if !exited {
        let _ = Command::new("kill")
            .arg("-KILL")
            .arg(format!("-{pid}"))
            .status();
        let _ = child.wait();
    }
    assert!(exited, "local process group should exit after task cancel");
    let result = stored_result_json(&state, &task_id);
    assert_eq!(result["cancel_adapter_result"]["status"], "accepted");
    assert_eq!(
        result["cancel_adapter_result"]["signal_scope"],
        "process_group_or_pid"
    );
    assert!(result["cancel_adapter_result"].get("text").is_none());
    assert!(result["cancel_adapter_result"].get("error_text").is_none());
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
fn resume_task_with_input_marks_checkpoint_due_without_restart() {
    let state = state_with_tasks_table();
    let task_id = Uuid::new_v4().to_string();
    let checkpoint_id = "ckpt-resume-now";
    let result = json!({
        "task_lifecycle": {
            "schema_version": 1,
            "state": "waiting",
            "source": "test",
            "checkpoint_id": checkpoint_id,
            "next_check_after": 4_000_000_000_i64
        },
        "task_checkpoint": checkpoint_json(checkpoint_id, vec!["side-effect-1"])
    });
    insert_task(&state, &task_id, "running", Some(&result), 1234);

    let update = resume_task_with_input(
        &state,
        TaskResumeControlInput {
            task_id: task_id.clone(),
            checkpoint_id: Some(checkpoint_id.to_string()),
            resume_trigger: crate::task_lifecycle::ResumeTrigger::UserFollowup,
            resume_reason: None,
            user_message: None,
            new_constraints: None,
        },
    )
    .expect("resume task")
    .expect("task resumable");

    assert_eq!(update.task_id, task_id);
    assert_eq!(update.checkpoint_id, checkpoint_id);
    assert_eq!(update.lifecycle["resume_due"], true);
    assert_eq!(update.lifecycle["resume_wait_seconds"], 0);
    assert_eq!(
        update.lifecycle["message_key"],
        "clawd.task.resume_requested"
    );
    let stored = stored_result_json(&state, &task_id);
    assert_eq!(stored["task_lifecycle"]["resume_due"], true);
    assert_eq!(stored["task_lifecycle"]["resume_wait_seconds"], 0);
    assert_eq!(
        stored["task_checkpoint"]["completed_side_effect_refs"][0],
        "side-effect-1"
    );
}

#[test]
fn pause_task_by_id_delays_existing_checkpoint_only() {
    let state = state_with_tasks_table();
    let task_id = Uuid::new_v4().to_string();
    let checkpoint_id = "ckpt-pause-existing";
    let result = json!({
        "task_lifecycle": {
            "schema_version": 1,
            "state": "background",
            "source": "test",
            "checkpoint_id": checkpoint_id,
            "next_check_after": 1
        },
        "task_checkpoint": checkpoint_json(checkpoint_id, vec![])
    });
    insert_task(&state, &task_id, "running", Some(&result), 1234);

    let update = pause_task_by_id(&state, &task_id, 120)
        .expect("pause task")
        .expect("task pauseable");

    assert_eq!(update.checkpoint_id, checkpoint_id);
    assert_eq!(update.lifecycle["resume_due"], false);
    assert_eq!(update.lifecycle["resume_wait_seconds"], 120);
    assert_eq!(
        update.lifecycle["message_key"],
        "clawd.task.pause_requested"
    );
}

#[test]
fn pause_task_by_id_rejects_running_task_without_checkpoint() {
    let state = state_with_tasks_table();
    let task_id = Uuid::new_v4().to_string();
    insert_task(
        &state,
        &task_id,
        "running",
        Some(&json!({"text": "working"})),
        1234,
    );

    let update = pause_task_by_id(&state, &task_id, 120).expect("pause task");

    assert!(update.is_none());
    let (status, error_text) = stored_task_status_and_error(&state, &task_id);
    assert_eq!(status, "running");
    assert_eq!(error_text, None);
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

#[test]
fn foreground_heartbeat_cannot_reclaim_a_published_checkpoint() {
    let state = state_with_tasks_table();
    let checkpoint = json!({
        "task_lifecycle": {
            "state": "waiting",
            "resume_reason": "task_budget_slice_exhausted",
            "next_check_after": 1,
            "checkpoint_id": "ckpt-paused"
        },
        "task_checkpoint": checkpoint_json("ckpt-paused", vec![])
    });
    insert_task(&state, "paused-heartbeat", "running", Some(&checkpoint), 1);

    assert!(
        !touch_running_task(&state, "paused-heartbeat", 0).expect("touch paused checkpoint"),
        "a foreground heartbeat must not reacquire a handed-off checkpoint"
    );
}

#[test]
fn claim_due_paused_checkpoint_task_marks_expired_checkpoint_lease_takeover() {
    let state = state_with_tasks_table();
    let now = 7_000;
    let due = json!({
        "task_lifecycle": {
            "state": "background",
            "resume_reason": "agent_loop_soft_budget",
            "next_check_after": now - 5,
            "checkpoint_id": "ckpt-expired-claim",
            "resume_claim": {
                "schema_version": 1,
                "owner": "worker:previous",
                "owner_layer": "worker_recovery",
                "checkpoint_id": "ckpt-expired-claim",
                "claimed_at": now - 70,
                "expires_at": now - 1
            }
        },
        "task_checkpoint": checkpoint_json("ckpt-expired-claim", vec!["external_call:job-1"])
    });
    insert_task(
        &state,
        "expired-checkpoint-claim",
        "running",
        Some(&due),
        100,
    );

    let claimed = claim_due_paused_checkpoint_task_internal(
        &state,
        "expired-checkpoint-claim",
        "ckpt-expired-claim",
        now,
        45,
    )
    .expect("claim expired checkpoint lease")
    .expect("expired checkpoint lease should be reclaimable");

    assert_eq!(claimed.task_id, "expired-checkpoint-claim");
    assert_eq!(claimed.completed_side_effect_count, 1);
    assert!(claimed.requires_idempotency_guard);

    let stored = stored_result_json(&state, "expired-checkpoint-claim");
    let lifecycle =
        crate::task_lifecycle::task_query_lifecycle_projection("running", Some(&stored), None);
    assert_eq!(
        lifecycle["resume_claim"]["checkpoint_id"],
        "ckpt-expired-claim"
    );
    assert_eq!(lifecycle["resume_claim"]["owner"], state.worker.worker_id);
    assert_eq!(
        lifecycle["resume_claim"]["recovery_reason"],
        "checkpoint_lease_expired"
    );
    assert_eq!(
        lifecycle["resume_claim"]["previous_claim_owner"],
        "worker:previous"
    );
    assert_eq!(
        lifecycle["resume_claim"]["previous_claim_expires_at"],
        now - 1
    );
    assert_eq!(lifecycle["resume_claim"]["expires_at"], now + 45);
}

#[path = "tasks_resume_executor_tests.rs"]
mod tasks_resume_executor_tests;

#[path = "task_resume_execution_tests.rs"]
mod task_resume_execution_tests;

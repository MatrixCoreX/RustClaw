use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::retry_child_task_with_revised_goal;

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "rustclaw_child_control_{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&path).expect("create temp directory");
        Self { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn state_with_schema(path: &Path) -> crate::AppState {
    let mut state = crate::AppState::test_default_with_fixture_provider();
    let manager = r2d2_sqlite::SqliteConnectionManager::file(path);
    state.core.db = r2d2::Pool::builder()
        .max_size(2)
        .build(manager)
        .expect("build test database pool");
    state.with_seeded_db_schema()
}

fn child_payload(parent_task_id: &str, child_task_id: &str) -> Value {
    json!({
        "text": "original objective",
        "task_role": "subagent_child",
        "parent_task_id": parent_task_id,
        "child_task_id": child_task_id,
        "child_task_contract": {
            "schema_version": 1,
            "parent_task_id": parent_task_id,
            "child_task_id": child_task_id,
            "role": "writer",
            "scope": {
                "objective": "original objective",
                "allowed_capabilities": [
                    "filesystem.read_text_range",
                    "workspace.apply_patch"
                ]
            },
            "permission_profile": "local_worktree",
            "required": true,
            "budget": {
                "max_rounds": 4,
                "max_tool_calls": 16,
                "timeout_ms": 300000
            },
            "result_contract": {
                "output_format": "machine_json"
            },
            "merge_policy": "structured_findings"
        }
    })
}

fn insert_task(
    state: &crate::AppState,
    task_id: &str,
    status: &str,
    payload: &Value,
    result: &Value,
) {
    let db = state.core.db.get().expect("get database");
    db.execute(
        "INSERT INTO tasks (
            task_id, user_id, chat_id, user_key, channel, kind, payload_json,
            status, result_json, error_text, created_at, updated_at
         )
         VALUES (?1, 42, 7, 'test-key', 'ui', 'ask', ?2, ?3, ?4, NULL, '1', '1')",
        rusqlite::params![task_id, payload.to_string(), status, result.to_string()],
    )
    .expect("insert task");
}

fn task_payload(state: &crate::AppState, task_id: &str) -> Value {
    task_json_column(state, task_id, "payload_json")
}

fn task_result(state: &crate::AppState, task_id: &str) -> Value {
    task_json_column(state, task_id, "result_json")
}

fn task_status(state: &crate::AppState, task_id: &str) -> String {
    let db = state.core.db.get().expect("get database");
    db.query_row(
        "SELECT status FROM tasks WHERE task_id = ?1",
        rusqlite::params![task_id],
        |row| row.get(0),
    )
    .expect("select task status")
}

fn task_json_column(state: &crate::AppState, task_id: &str, column: &str) -> Value {
    assert!(matches!(column, "payload_json" | "result_json"));
    let db = state.core.db.get().expect("get database");
    let query = format!("SELECT {column} FROM tasks WHERE task_id = ?1");
    let raw: String = db
        .query_row(&query, rusqlite::params![task_id], |row| row.get(0))
        .expect("select task JSON");
    serde_json::from_str(&raw).expect("parse task JSON")
}

fn paused_child_result(checkpoint_id: &str) -> Value {
    json!({
        "task_lifecycle": {
            "schema_version": 1,
            "state": "waiting",
            "checkpoint_id": checkpoint_id,
            "next_check_after": crate::now_ts_u64().saturating_add(300),
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
fn failed_child_retry_preserves_contract_and_supersedes_previous_attempt() {
    let temp = TempDir::new();
    let state = state_with_schema(&temp.path.join("tasks.sqlite"));
    let parent_task_id = "parent-retry";
    let child_task_id = "child-retry-failed";
    insert_task(
        &state,
        parent_task_id,
        "running",
        &json!({"text": "parent"}),
        &json!({"child_task_ids": [child_task_id]}),
    );
    insert_task(
        &state,
        child_task_id,
        "failed",
        &child_payload(parent_task_id, child_task_id),
        &json!({"status_code": "verification_failed"}),
    );

    let update = retry_child_task_with_revised_goal(
        &state,
        parent_task_id,
        child_task_id,
        "repair the failing verification without changing the public API",
    )
    .expect("retry child")
    .expect("retry update");

    assert_eq!(update.previous_child_task_id, child_task_id);
    assert_eq!(update.retry_index, 1);
    assert_eq!(update.lifecycle["state"], "queued");
    let payload = task_payload(&state, &update.child_task_id);
    assert_eq!(
        payload["text"],
        "repair the failing verification without changing the public API"
    );
    assert_eq!(
        payload["child_task_contract"]["scope"]["objective"],
        payload["text"]
    );
    assert_eq!(
        payload["child_task_contract"]["permission_profile"],
        "local_worktree"
    );
    assert_eq!(
        payload["child_task_contract"]["scope"]["allowed_capabilities"][1],
        "workspace.apply_patch"
    );
    assert_eq!(
        payload["child_task_contract"]["budget"]["max_tool_calls"],
        16
    );

    let parent = task_result(&state, parent_task_id);
    assert_eq!(parent["superseded_child_task_ids"][0], child_task_id);
    assert_eq!(
        parent["child_task_retries"][0]["child_task_id"],
        update.child_task_id
    );
    assert_eq!(
        parent["child_task_merge"]["parent_continuation"]["status"],
        "waiting"
    );
    assert_eq!(
        parent["child_task_merge"]["child_task_ids"],
        json!([update.child_task_id])
    );
}

#[test]
fn successful_retry_unblocks_parent_without_recounting_required_failure() {
    let temp = TempDir::new();
    let state = state_with_schema(&temp.path.join("tasks.sqlite"));
    let parent_task_id = "parent-retry-success";
    let child_task_id = "child-retry-required-failure";
    insert_task(
        &state,
        parent_task_id,
        "running",
        &json!({"text": "parent"}),
        &json!({"child_task_ids": [child_task_id]}),
    );
    insert_task(
        &state,
        child_task_id,
        "failed",
        &child_payload(parent_task_id, child_task_id),
        &json!({"status_code": "verification_failed"}),
    );
    let update =
        retry_child_task_with_revised_goal(&state, parent_task_id, child_task_id, "revised goal")
            .expect("retry child")
            .expect("retry update");
    let retry_payload = task_payload(&state, &update.child_task_id);
    {
        let db = state.core.db.get().expect("get database");
        db.execute(
            "UPDATE tasks
             SET status = 'succeeded', result_json = ?2
             WHERE task_id = ?1",
            rusqlite::params![
                update.child_task_id,
                json!({"status_code": "completed"}).to_string()
            ],
        )
        .expect("complete retry");
    }
    assert!(
        crate::repo::child_tasks::record_child_task_terminal_projection(
            &state,
            &update.child_task_id,
            &retry_payload,
        )
        .expect("record retry projection")
    );

    let parent = task_result(&state, parent_task_id);
    assert_eq!(
        parent["child_task_merge"]["parent_continuation"]["status"],
        "ready"
    );
    assert_eq!(
        parent["child_task_merge"]["merge"]["required_failed_count"],
        0
    );
    assert_eq!(parent["child_task_merge"]["merge"]["completed_count"], 1);
}

#[test]
fn retry_rejects_active_child_without_mutating_parent() {
    let temp = TempDir::new();
    let state = state_with_schema(&temp.path.join("tasks.sqlite"));
    let parent_task_id = "parent-active-child";
    let child_task_id = "child-still-running";
    insert_task(
        &state,
        parent_task_id,
        "running",
        &json!({"text": "parent"}),
        &json!({"child_task_ids": [child_task_id]}),
    );
    insert_task(
        &state,
        child_task_id,
        "running",
        &child_payload(parent_task_id, child_task_id),
        &json!({}),
    );

    let error = retry_child_task_with_revised_goal(
        &state,
        parent_task_id,
        child_task_id,
        "replacement goal",
    )
    .expect_err("running child must not retry");

    assert_eq!(error.to_string(), "child_task_not_retryable");
    assert_eq!(
        task_result(&state, parent_task_id)["child_task_ids"],
        json!([child_task_id])
    );
}

#[test]
fn child_pause_steer_resume_and_cancel_are_task_scoped() {
    let temp = TempDir::new();
    let state = state_with_schema(&temp.path.join("tasks.sqlite"));
    let parent_task_id = "parent-child-controls";
    let steered_child_id = "child-controls-steered";
    let canceled_child_id = "child-controls-canceled";
    insert_task(
        &state,
        parent_task_id,
        "running",
        &json!({"text": "parent"}),
        &json!({"child_task_ids": [steered_child_id, canceled_child_id]}),
    );
    insert_task(
        &state,
        steered_child_id,
        "running",
        &child_payload(parent_task_id, steered_child_id),
        &paused_child_result("ckpt-child-steer"),
    );
    insert_task(
        &state,
        canceled_child_id,
        "running",
        &child_payload(parent_task_id, canceled_child_id),
        &json!({}),
    );

    let paused = crate::repo::pause_task_by_id(&state, steered_child_id, 60)
        .expect("pause child")
        .expect("pause update");
    assert_eq!(paused.lifecycle["control_request"]["kind"], "pause");
    assert_eq!(task_status(&state, canceled_child_id), "running");

    let resumed = crate::repo::resume_task_with_input(
        &state,
        crate::repo::TaskResumeControlInput {
            task_id: steered_child_id.to_string(),
            checkpoint_id: Some("ckpt-child-steer".to_string()),
            resume_trigger: crate::task_lifecycle::ResumeTrigger::UserFollowup,
            resume_reason: Some("child_goal_steered".to_string()),
            user_message: Some("继续验证，不要扩大写入范围".to_string()),
            new_constraints: Some(json!({
                "allowed_capabilities": ["filesystem.read_text_range"]
            })),
        },
    )
    .expect("resume child")
    .expect("resume update");
    assert_eq!(resumed.lifecycle["control_request"]["kind"], "resume");
    assert_eq!(
        resumed.lifecycle["resume_input"]["user_message"],
        "继续验证，不要扩大写入范围"
    );
    assert_eq!(
        resumed.lifecycle["resume_input"]["new_constraints"]["allowed_capabilities"],
        json!(["filesystem.read_text_range"])
    );

    assert_eq!(
        crate::repo::cancel_task_by_id(&state, canceled_child_id).expect("cancel sibling child"),
        1
    );
    assert_eq!(task_status(&state, canceled_child_id), "canceled");
    assert_eq!(task_status(&state, steered_child_id), "running");
    assert_eq!(task_status(&state, parent_task_id), "running");
}

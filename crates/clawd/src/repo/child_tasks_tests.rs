use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use serde_json::{json, Value};

use super::*;
use crate::child_task_contract::{
    ChildTaskBudget, ChildTaskMergePolicy, ChildTaskPermissionProfile,
};

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

fn sample_child_spec(parent_task_id: &str, child_task_id: &str, required: bool) -> ChildTaskSpec {
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

fn child_payload(spec: &ChildTaskSpec) -> Value {
    json!({
        "text": "visible child objective",
        "task_role": "subagent_child",
        "parent_task_id": spec.parent_task_id,
        "child_task_id": spec.child_task_id,
        "child_task_contract": spec.to_json()
    })
}

fn insert_task(
    state: &crate::AppState,
    task_id: &str,
    status: &str,
    payload: &Value,
    result: &Value,
) {
    let db = state.core.db.get().expect("get db");
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

#[test]
fn child_timeout_projection_blocks_required_parent_after_restart() {
    let temp = TempDirGuard::new("child_timeout_restart");
    let db_path = temp.path.join("tasks.sqlite");
    let first_state = file_backed_state_with_schema(&db_path);
    let parent_payload = json!({"text": "visible parent request"});
    insert_task(
        &first_state,
        "task-parent-timeout-restart",
        "running",
        &parent_payload,
        &json!({
            "child_task_ids": ["task-child-timeout-restart"],
            "text": "visible parent prose"
        }),
    );
    let spec = sample_child_spec(
        "task-parent-timeout-restart",
        "task-child-timeout-restart",
        true,
    );
    let payload = child_payload(&spec);
    insert_task(
        &first_state,
        "task-child-timeout-restart",
        "timeout",
        &payload,
        &json!({
            "text": "visible child timeout prose",
            "error_text": "visible child timeout error"
        }),
    );
    drop(first_state);

    let restarted_state = file_backed_state_with_schema(&db_path);
    assert!(record_child_task_terminal_projection(
        &restarted_state,
        "task-child-timeout-restart",
        &payload,
    )
    .expect("record child timeout projection after restart"));

    let child = stored_result_json(&restarted_state, "task-child-timeout-restart");
    assert_eq!(child["child_task_result"]["status"], "failed");
    assert_eq!(child["child_task_result"]["required"], true);
    assert!(child["child_task_result"].get("text").is_none());
    assert!(child["child_task_result"].get("error_text").is_none());

    let parent = stored_result_json(&restarted_state, "task-parent-timeout-restart");
    let merge = &parent["child_task_merge"];
    assert_eq!(merge["parent_continuation"]["status"], "blocked");
    assert_eq!(
        merge["parent_continuation"]["reason_code"],
        "required_child_failed"
    );
    assert_eq!(merge["merge"]["required_failed_count"], 1);
    assert_eq!(merge["merge"]["parent_can_continue"], false);
    assert!(merge
        .to_string()
        .find("visible child timeout prose")
        .is_none());
    assert!(merge
        .to_string()
        .find("visible child timeout error")
        .is_none());
}

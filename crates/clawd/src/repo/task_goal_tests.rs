use serde_json::json;
use uuid::Uuid;

use super::{task_goal_projection, update_task_goal_payload, TaskGoalControlOperation};

fn state_with_goal_task(task_id: &str, payload: serde_json::Value) -> crate::AppState {
    let state = crate::AppState::test_default_with_fixture_provider();
    let db = state.core.db.get().expect("get db");
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS tasks (
            task_id TEXT PRIMARY KEY,
            payload_json TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );",
    )
    .expect("create tasks table");
    db.execute(
        "INSERT OR REPLACE INTO tasks (task_id, payload_json, updated_at)
         VALUES (?1, ?2, '0')",
        rusqlite::params![task_id, payload.to_string()],
    )
    .expect("insert task");
    drop(db);
    state
}

#[test]
fn task_goal_projection_merges_payload_goal_and_structured_progress() {
    let task_id = Uuid::parse_str("00000000-0000-0000-0000-000000000123").unwrap();
    let payload = json!({
        "text": "update workspace",
        "goal": {
            "objective": "update workspace",
            "done_conditions": ["code_changed", "tests_pass"],
            "verification_commands": ["cargo test -p clawcli"],
            "constraints": [{"scope": "workspace"}]
        }
    });
    let result = json!({
        "task_journal": {
            "summary": {
                "task_goal": {
                    "goal_status": "completed",
                    "goal_status_source": "journal_final_status",
                    "current_progress": ["changed_file_count=2"],
                    "remaining_work": [],
                    "success_evidence_refs": ["event:task_completed"],
                    "validation_status": "passed"
                }
            }
        }
    });
    let lifecycle = json!({
        "state": "completed",
        "execution_state": "completed"
    });

    let goal =
        task_goal_projection(task_id, &payload.to_string(), Some(&result), &lifecycle).unwrap();

    assert_eq!(goal["schema_version"], 1);
    assert_eq!(goal["task_id"], task_id.to_string());
    assert_eq!(goal["goal_id"], format!("task:{task_id}"));
    assert_eq!(goal["objective"], "update workspace");
    assert_eq!(goal["done_conditions"][1], "tests_pass");
    assert_eq!(goal["verification_commands"][0], "cargo test -p clawcli");
    assert_eq!(goal["constraints"][0]["scope"], "workspace");
    assert_eq!(goal["goal_status"], "completed");
    assert_eq!(goal["goal_status_source"], "journal_final_status");
    assert_eq!(goal["validation_status"], "passed");
    assert_eq!(goal["current_progress"][0], "changed_file_count=2");
    assert_eq!(goal["success_evidence_refs"][0], "event:task_completed");
}

#[test]
fn task_goal_projection_uses_lifecycle_status_without_text_matching() {
    let task_id = Uuid::parse_str("00000000-0000-0000-0000-000000000124").unwrap();
    let payload = json!({
        "goal_spec": {
            "objective": "background workflow",
            "done_conditions": ["checkpoint_ready"]
        }
    });
    let lifecycle = json!({
        "execution_state": "background",
        "checkpoint_id": "ckpt-1"
    });

    let goal = task_goal_projection(task_id, &payload.to_string(), None, &lifecycle).unwrap();

    assert_eq!(goal["goal_status"], "background");
    assert_eq!(goal["goal_status_source"], "lifecycle");
    assert_eq!(goal["objective"], "background workflow");
}

#[test]
fn task_goal_projection_returns_none_without_goal_sources() {
    let task_id = Uuid::parse_str("00000000-0000-0000-0000-000000000125").unwrap();
    let lifecycle = json!({"execution_state": "completed"});

    assert!(task_goal_projection(task_id, r#"{"text":"plain"}"#, None, &lifecycle).is_none());
}

#[test]
fn update_task_goal_payload_edits_structured_goal_fields() {
    let task_id = "task-goal-edit";
    let state = state_with_goal_task(
        task_id,
        json!({
            "text": "work",
            "goal": {
                "objective": "old",
                "done_conditions": ["old_done"]
            },
            "goal_spec": {
                "objective": "legacy"
            }
        }),
    );

    let update = update_task_goal_payload(
        &state,
        task_id,
        TaskGoalControlOperation::Edit,
        Some(json!({
            "objective": "new",
            "verification_commands": ["cargo test -p clawcli"],
            "constraints": ["scope=workspace"]
        })),
    )
    .expect("edit goal")
    .expect("goal update");

    assert_eq!(update.operation, "edit");
    assert_eq!(update.goal.as_ref().unwrap()["objective"], "new");
    assert_eq!(
        update.goal.as_ref().unwrap()["verification_commands"][0],
        "cargo test -p clawcli"
    );
    assert!(update.payload_json.get("goal_spec").is_none());
    assert_eq!(
        update.payload_json["goal"]["done_conditions"][0],
        "old_done"
    );
    assert_eq!(update.payload_json["goal"]["schema_version"], 1);
}

#[test]
fn update_task_goal_payload_normalizes_goal_status_machine_token() {
    let task_id = "task-goal-status";
    let state = state_with_goal_task(
        task_id,
        json!({
            "text": "work",
            "goal": {"objective": "old"}
        }),
    );

    let update = update_task_goal_payload(
        &state,
        task_id,
        TaskGoalControlOperation::Edit,
        Some(json!({
            "goal_status": "canceled"
        })),
    )
    .expect("edit status")
    .expect("goal update");

    assert_eq!(update.goal.as_ref().unwrap()["goal_status"], "cancelled");
}

#[test]
fn update_task_goal_payload_rejects_invalid_goal_status_token() {
    let task_id = "task-goal-status-invalid";
    let state = state_with_goal_task(
        task_id,
        json!({
            "text": "work",
            "goal": {"objective": "old"}
        }),
    );

    let err = update_task_goal_payload(
        &state,
        task_id,
        TaskGoalControlOperation::Edit,
        Some(json!({
            "goal_status": "done soon"
        })),
    )
    .expect_err("invalid status should fail");

    assert!(err.to_string().contains("goal_status_invalid"));
}

#[test]
fn task_goal_projection_ignores_invalid_result_goal_status() {
    let task_id = Uuid::parse_str("00000000-0000-0000-0000-000000000126").unwrap();
    let payload = json!({
        "goal": {
            "objective": "status fallback"
        }
    });
    let result = json!({
        "task_journal": {
            "summary": {
                "task_goal": {
                    "goal_status": "done soon",
                    "goal_status_source": "journal_final_status"
                }
            }
        }
    });
    let lifecycle = json!({"execution_state": "running"});

    let goal =
        task_goal_projection(task_id, &payload.to_string(), Some(&result), &lifecycle).unwrap();

    assert_eq!(goal["goal_status"], "in_progress");
    assert_eq!(goal["goal_status_source"], "lifecycle");
}

#[test]
fn update_task_goal_payload_clears_goal_keys() {
    let task_id = "task-goal-clear";
    let state = state_with_goal_task(
        task_id,
        json!({
            "text": "work",
            "goal": {"objective": "old"},
            "task_goal": {"objective": "legacy"}
        }),
    );

    let update = update_task_goal_payload(&state, task_id, TaskGoalControlOperation::Clear, None)
        .expect("clear goal")
        .expect("goal update");

    assert_eq!(update.operation, "clear");
    assert!(update.goal.is_none());
    assert!(update.payload_json.get("goal").is_none());
    assert!(update.payload_json.get("task_goal").is_none());
    assert_eq!(update.payload_json["text"], "work");
}

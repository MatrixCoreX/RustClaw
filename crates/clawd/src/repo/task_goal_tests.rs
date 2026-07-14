use serde_json::json;
use uuid::Uuid;

use super::task_goal_projection;

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
                "task_outcome": {
                    "goal_status": "completed",
                    "current_progress": ["changed_file_count=2"],
                    "remaining_work": [],
                    "success_evidence_refs": ["event:task_completed"]
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

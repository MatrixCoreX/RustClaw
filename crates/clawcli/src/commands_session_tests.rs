use super::{
    session_list_json, session_resume_json, session_show_json, session_store_archive_json,
    session_store_delete_json, session_store_fork_json, session_store_upsert_summary, SessionStore,
};

#[test]
fn session_list_json_indexes_active_task_machine_fields() {
    let active = serde_json::json!({
        "data": {
            "tasks": [
                {
                    "task_id": "task-session-a",
                    "status": "running",
                    "execution_state": "background",
                    "task_lifecycle": {
                        "state": "background",
                        "checkpoint_id": "ckpt-session"
                    },
                    "goal": {
                        "goal_id": "goal-session"
                    },
                    "latest_event_seq": "42"
                }
            ]
        }
    });

    let summary = session_list_json(7, 9, &active);

    assert_eq!(summary["session_kind"], "user_chat_active_tasks");
    assert_eq!(summary["session_id"], "user_chat:7:9");
    assert_eq!(summary["task_count"], 1);
    assert_eq!(summary["task_ids"][0], "task-session-a");
    assert_eq!(summary["active_goal_id"], "goal-session");
    assert_eq!(summary["latest_checkpoint_id"], "ckpt-session");
    assert_eq!(summary["tasks"][0]["lifecycle_state"], "background");
}

#[test]
fn session_show_json_wraps_task_goal_checkpoint_and_report() {
    let selected = crate::task::TaskStatusView {
        task_id: "task-session-show".to_string(),
        status: "running".to_string(),
        raw_data: serde_json::json!({
            "execution_state": "background",
            "task_lifecycle": {
                "state": "background",
                "checkpoint_id": "ckpt-show"
            },
            "goal": {
                "goal_id": "goal-show"
            },
            "result_json": {
                "changed_files": ["src/lib.rs"]
            }
        }),
        result_text: None,
        error_text: None,
        events: vec![crate::events::TaskEventLine {
            event_type: "task_progress".to_string(),
            line: "event_seq=11".to_string(),
            fields: std::collections::BTreeMap::from([("event_seq".to_string(), "11".to_string())]),
        }],
    };

    let summary = session_show_json(&selected);

    assert_eq!(summary["session_kind"], "task_session");
    assert_eq!(summary["session_id"], "task-session-show");
    assert_eq!(summary["active_goal_id"], "goal-show");
    assert_eq!(summary["latest_checkpoint_id"], "ckpt-show");
    assert_eq!(summary["latest_event_seq"], "11");
    assert_eq!(summary["summary"]["coding"]["changed_file_count"], 1);
}

#[test]
fn session_resume_json_extracts_machine_resume_fields() {
    let body = serde_json::json!({
        "data": {
            "task_id": "task-session-resume",
            "status": "running",
            "task_lifecycle": {
                "state": "background",
                "checkpoint_id": "ckpt-resume",
                "resume_due": true,
                "resume_reason": "checkpoint_wait",
                "next_action_kind": "resume_checkpoint"
            }
        }
    });

    let summary = session_resume_json("task-session-resume", &body);

    assert_eq!(summary["operation"], "session_resume");
    assert_eq!(summary["session_id"], "task-session-resume");
    assert_eq!(summary["checkpoint_id"], "ckpt-resume");
    assert_eq!(summary["resume_due"], true);
    assert_eq!(summary["next_action_kind"], "resume_checkpoint");
}

#[test]
fn session_store_archive_delete_and_fork_use_machine_metadata() {
    let mut store = SessionStore::default();
    let summary = serde_json::json!({
        "session_id": "task-session-store",
        "task_ids": ["task-session-store"],
        "active_goal_id": "goal-store",
        "workspace_root": "/tmp/rustclaw",
        "latest_checkpoint_id": "ckpt-store",
        "latest_event_seq": "77",
        "archived": false
    });

    let upsert = session_store_upsert_summary(&mut store, &summary);
    assert_eq!(upsert["operation"], "session_store_upsert");
    assert_eq!(upsert["status"], "ok");

    let archive = session_store_archive_json(&mut store, "task-session-store");
    assert_eq!(archive["operation"], "session_archive");
    assert_eq!(archive["archived"], true);
    assert_eq!(archive["store_session_count"], 1);

    let fork = session_store_fork_json(&mut store, "task-session-store", "task-session-fork")
        .expect("fork session metadata");
    assert_eq!(fork["operation"], "session_fork");
    assert_eq!(fork["session_id"], "task-session-fork");
    assert_eq!(fork["forked_from"], "task-session-store");
    assert_eq!(fork["store_session_count"], 2);

    let delete = session_store_delete_json(&mut store, "task-session-store");
    assert_eq!(delete["operation"], "session_delete");
    assert_eq!(delete["deleted"], true);
    assert_eq!(delete["store_session_count"], 1);
}

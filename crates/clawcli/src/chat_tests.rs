use super::{goal_projection, session_submission_options};
use crate::chat_session::{ChatSessionState, PermissionMode, WorkingDirectoryIdentity};

fn session(mode: PermissionMode) -> ChatSessionState {
    ChatSessionState {
        conversation_id: "conversation-1".to_string(),
        session_id: "conversation-1".to_string(),
        active_task_id: None,
        task_ids: Vec::new(),
        model_override: None,
        permission_mode: mode,
        attachments: Vec::new(),
        compacted_context_ref: None,
        goal_ref: None,
        event_cursor: 0,
        working_directory: WorkingDirectoryIdentity {
            canonical_path: "/tmp".to_string(),
            identity_sha256: "digest".to_string(),
        },
    }
}

#[test]
fn session_permission_mode_drives_each_submission() {
    for mode in [
        PermissionMode::Safe,
        PermissionMode::Ask,
        PermissionMode::Yolo,
    ] {
        let options = session_submission_options(&session(mode));
        assert!(!options.yolo);
        assert_eq!(options.permission_mode, Some(mode));
    }
}

#[test]
fn goal_projection_prefers_authoritative_task_goal() {
    let mut session = session(PermissionMode::Ask);
    session.active_task_id = Some("task-1".to_string());
    session.goal_ref = Some("goal:stale".to_string());
    let task = crate::task::TaskStatusView {
        task_id: "task-1".to_string(),
        status: "running".to_string(),
        raw_data: serde_json::json!({
            "task_goal": {
                "goal_id": "goal:authoritative",
                "status": "active",
                "remaining_steps": 2
            }
        }),
        result_text: None,
        error_text: None,
        events: Vec::new(),
    };

    let projection = goal_projection(&session, Some(&task));

    assert_eq!(projection["source"], "server_task");
    assert_eq!(projection["goal_ref"], "goal:authoritative");
    assert_eq!(projection["goal"]["remaining_steps"], 2);
}

#[test]
fn goal_projection_without_task_uses_safe_session_reference() {
    let mut session = session(PermissionMode::Ask);
    session.goal_ref = Some("goal:session".to_string());

    let projection = goal_projection(&session, None);

    assert_eq!(projection["source"], "session_reference");
    assert_eq!(projection["goal_ref"], "goal:session");
    assert!(projection["goal"].is_null());
}

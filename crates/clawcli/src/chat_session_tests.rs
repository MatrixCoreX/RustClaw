use super::{
    working_directory_identity, ChatSessionState, ChatSessionTransition, PermissionMode,
    WorkingDirectoryIdentity,
};

fn state() -> ChatSessionState {
    ChatSessionState {
        conversation_id: "conversation-1".to_string(),
        session_id: "conversation-1".to_string(),
        active_task_id: None,
        task_ids: Vec::new(),
        model_override: None,
        permission_mode: PermissionMode::Ask,
        attachments: Vec::new(),
        compacted_context_ref: None,
        goal_ref: None,
        rewind_anchor: None,
        completed_side_effect_refs: Vec::new(),
        event_cursor: 0,
        working_directory: WorkingDirectoryIdentity {
            canonical_path: "/tmp/workspace".to_string(),
            identity_sha256: "digest".to_string(),
        },
    }
}

#[test]
fn typed_transitions_preserve_task_and_cursor_invariants() {
    let mut state = state();
    state
        .apply(ChatSessionTransition::TaskSubmitted(
            "task-first".to_string(),
        ))
        .unwrap();
    state
        .apply(ChatSessionTransition::CursorAdvanced(12))
        .unwrap();
    assert_eq!(state.active_task_id.as_deref(), Some("task-first"));
    assert_eq!(state.task_ids, ["task-first"]);
    assert_eq!(state.event_cursor, 12);
    assert!(state
        .apply(ChatSessionTransition::CursorAdvanced(11))
        .is_err());

    state
        .apply(ChatSessionTransition::TaskSelected(
            "task-second".to_string(),
        ))
        .unwrap();
    assert_eq!(state.active_task_id.as_deref(), Some("task-second"));
    assert_eq!(state.event_cursor, 0);
}

#[test]
fn session_preferences_are_typed() {
    let mut state = state();
    state
        .apply(ChatSessionTransition::PermissionChanged(
            PermissionMode::Safe,
        ))
        .unwrap();
    state
        .apply(ChatSessionTransition::ContextCompacted(
            "context:42".to_string(),
        ))
        .unwrap();
    state.goal_ref = Some("goal:active".to_string());
    assert_eq!(state.permission_mode, PermissionMode::Safe);
    assert_eq!(state.compacted_context_ref.as_deref(), Some("context:42"));
    assert_eq!(state.goal_ref.as_deref(), Some("goal:active"));
}

#[test]
fn working_directory_identity_is_canonical_and_stable() {
    let first = working_directory_identity(std::path::Path::new(".")).unwrap();
    let second = working_directory_identity(std::path::Path::new(".")).unwrap();
    assert_eq!(first, second);
    assert!(!first.canonical_path.is_empty());
    assert_eq!(first.identity_sha256.len(), 64);
}

use super::{
    session_list_json, session_resume_json, session_show_json, session_store_archive_json,
    session_store_delete_json, session_store_fork_json, session_store_persist_chat_session,
    session_store_record_chat_cursor, session_store_record_chat_task, session_store_rewind_json,
    session_store_select_chat_session, session_store_select_latest_chat_session,
    session_store_upsert_summary, SessionStore,
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
        "workspace_root": "/tmp/agent-runtime",
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

#[test]
fn session_rewind_forks_a_new_thread_and_preserves_side_effect_refs() {
    let mut store = SessionStore::default();
    session_store_upsert_summary(
        &mut store,
        &serde_json::json!({
            "session_id":"source-session",
            "thread_id":"source-session",
            "task_ids":["task-1"],
            "latest_event_seq":"19"
        }),
    );
    let anchor = serde_json::json!({
        "schema_version":1,
        "source_session_id":"source-session",
        "source_task_id":"task-1",
        "event_seq":12,
        "checkpoint_id":"checkpoint-9"
    });
    let summary = session_store_rewind_json(
        &mut store,
        "source-session",
        "rewound-session",
        anchor.clone(),
        vec!["mutation:already-completed".to_string()],
    )
    .expect("rewind session");

    assert_eq!(summary["operation"], "session_rewind");
    assert_eq!(summary["original_history_preserved"], true);
    let rewound = session_store_select_latest_chat_session(&store).unwrap();
    assert_eq!(rewound.conversation_id, "rewound-session");
    assert_eq!(rewound.rewind_anchor, Some(anchor));
    assert_eq!(
        rewound.completed_side_effect_refs,
        vec!["mutation:already-completed"]
    );
    assert_eq!(rewound.event_cursor, 12);
}

#[test]
fn chat_session_store_resumes_latest_and_persists_task_cursor() {
    let mut store = SessionStore::default();
    let mut first =
        session_store_select_chat_session(&mut store, None, false, "cli_conversation_generated_1")
            .expect("create chat session");
    assert_eq!(first.conversation_id, "cli_conversation_generated_1");
    assert!(first.active_task_id.is_none());

    session_store_record_chat_task(&mut store, &mut first, "task-first").expect("record task");
    session_store_record_chat_cursor(&mut store, &mut first, 17).expect("record cursor");
    assert_eq!(first.active_task_id.as_deref(), Some("task-first"));
    assert_eq!(first.event_cursor, 17);

    let resumed =
        session_store_select_chat_session(&mut store, None, false, "cli_conversation_generated_2")
            .expect("resume latest session");
    assert_eq!(resumed.conversation_id, first.conversation_id);
    assert_eq!(resumed.active_task_id, first.active_task_id);
    assert_eq!(resumed.event_cursor, 17);
    let latest = session_store_select_latest_chat_session(&store).expect("select latest session");
    assert_eq!(latest, resumed);

    let fresh =
        session_store_select_chat_session(&mut store, None, true, "cli_conversation_generated_3")
            .expect("create fresh session");
    assert_eq!(fresh.conversation_id, "cli_conversation_generated_3");
    assert!(fresh.active_task_id.is_none());
}

#[test]
fn chat_session_store_rejects_non_machine_conversation_and_task_refs() {
    let mut store = SessionStore::default();
    assert!(session_store_select_latest_chat_session(&store).is_err());
    assert!(session_store_select_chat_session(
        &mut store,
        Some("conversation with spaces"),
        false,
        "unused"
    )
    .is_err());
    let mut state =
        session_store_select_chat_session(&mut store, None, false, "cli_conversation_valid")
            .unwrap();
    assert!(session_store_record_chat_task(&mut store, &mut state, "task/invalid").is_err());
}

#[test]
fn chat_session_store_persists_only_safe_preferences_and_authoritative_refs() {
    let mut store = SessionStore::default();
    let mut state =
        session_store_select_chat_session(&mut store, None, false, "cli_conversation_prefs")
            .unwrap();
    state.model_override = Some(crate::chat_session::ModelOverride {
        provider: "minimax".to_string(),
        model: "MiniMax-M3".to_string(),
    });
    state.permission_mode = crate::chat_session::PermissionMode::Safe;
    state.compacted_context_ref = Some("context:1".to_string());
    state.goal_ref = Some("goal:1".to_string());
    state
        .attachments
        .push(crate::chat_session::SessionAttachmentRef {
            canonical_path: state.working_directory.canonical_path.clone(),
            display_path: "docs/input.md".to_string(),
            kind: "file".to_string(),
            mime_type: "text/markdown".to_string(),
            size: 12,
            sha256: "a".repeat(64),
            materialization: "bounded_text_context".to_string(),
            truncated: false,
        });
    session_store_persist_chat_session(&mut store, &state).unwrap();

    let restored =
        session_store_select_latest_chat_session(&store).expect("restore safe preferences");
    assert_eq!(restored.model_override, state.model_override);
    assert_eq!(restored.permission_mode, state.permission_mode);
    assert_eq!(restored.compacted_context_ref, state.compacted_context_ref);
    assert_eq!(restored.goal_ref, state.goal_ref);
    assert_eq!(restored.attachments, state.attachments);

    let encoded = serde_json::to_value(&store).unwrap().to_string();
    assert!(!encoded.contains("prompt"));
    assert!(!encoded.contains("api_key"));
}

#[test]
fn successful_noninteractive_continuation_clears_persisted_pending_attachments() {
    let mut store = SessionStore::default();
    let mut state =
        session_store_select_chat_session(&mut store, None, false, "cli_conversation_pending")
            .unwrap();
    state
        .attachments
        .push(crate::chat_session::SessionAttachmentRef {
            canonical_path: state.working_directory.canonical_path.clone(),
            display_path: "docs/pending.md".to_string(),
            kind: "file".to_string(),
            mime_type: "text/markdown".to_string(),
            size: 12,
            sha256: "b".repeat(64),
            materialization: "bounded_text_context".to_string(),
            truncated: false,
        });
    session_store_persist_chat_session(&mut store, &state).unwrap();
    assert_eq!(
        session_store_select_latest_chat_session(&store)
            .unwrap()
            .attachments
            .len(),
        1
    );

    state
        .apply(crate::chat_session::ChatSessionTransition::AttachmentsCleared)
        .unwrap();
    session_store_record_chat_task(&mut store, &mut state, "task-continuation").unwrap();
    session_store_persist_chat_session(&mut store, &state).unwrap();

    let restored = session_store_select_latest_chat_session(&store).unwrap();
    assert!(restored.attachments.is_empty());
    assert_eq!(
        restored.active_task_id.as_deref(),
        Some("task-continuation")
    );
}

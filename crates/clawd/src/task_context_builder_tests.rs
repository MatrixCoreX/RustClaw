use super::{
    apply_execution_context_to_prompts, build_active_execution_anchor_context,
    build_active_task_context, build_session_alias_context, build_task_goal_context,
    session_snapshot_provides_execution_state_anchor, ExecutionContextBudgetTier,
    ExecutionContextView, PlannerContextView, TaskContextBundle, TaskContextRawSources,
};

fn empty_snapshot() -> crate::conversation_state::ActiveSessionSnapshot {
    crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    }
}

fn empty_prompt_memory_context() -> crate::memory::service::PromptMemoryContext {
    crate::memory::service::PromptMemoryContext {
        prompt_with_memory: String::new(),
        chat_prompt_context: String::new(),
        memory_trace: None,
        long_term_summary: None,
        preferences: Vec::new(),
        recalled: Vec::new(),
        similar_triggers: Vec::new(),
        relevant_facts: Vec::new(),
        recent_related_events: Vec::new(),
    }
}

#[test]
fn active_task_context_is_empty_without_primary_task_state() {
    assert_eq!(build_active_task_context(&empty_snapshot()), "<none>");
}

#[test]
fn active_task_context_includes_and_bounds_primary_turn() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("prepare a deployment note".to_string()),
            last_primary_task_output: Some("x".repeat(1100)),
            ..Default::default()
        }),
        ..empty_snapshot()
    };

    let context = build_active_task_context(&snapshot);
    assert!(context.contains("### ACTIVE_TASK_CONTEXT"));
    assert!(context.contains("prepare a deployment note"));
    assert!(context.contains("...(truncated)"));
    assert!(context.len() < 1500);
}

#[test]
fn session_alias_context_exports_structured_bindings() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            alias_bindings: vec![crate::conversation_state::SessionAliasBinding {
                alias: "report_ref".to_string(),
                target: "/workspace/report.txt".to_string(),
                updated_at_ts: 1,
            }],
            ..Default::default()
        }),
        ..empty_snapshot()
    };

    let context = build_session_alias_context(&snapshot);
    assert!(context.contains("### SESSION_ALIAS_BINDINGS"));
    assert!(context.contains("report_ref"));
    assert!(context.contains("/workspace/report.txt"));
}

#[test]
fn generic_followup_does_not_become_execution_evidence() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            source_request: "continue".to_string(),
            op_kind: crate::followup_frame::FollowupOpKind::Generic,
            bound_target: Some("/workspace/plan.md".to_string()),
            ordered_entries: vec!["plan.md".to_string()],
            ..Default::default()
        }),
        ..empty_snapshot()
    };

    let context = build_active_execution_anchor_context(&snapshot);
    assert!(context.contains("followup_op_kind: Generic"));
    assert!(!context.contains("followup_bound_target"));
    assert!(!session_snapshot_provides_execution_state_anchor(&snapshot));
}

#[test]
fn read_followup_exports_structured_execution_anchor() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            source_request: "read".to_string(),
            op_kind: crate::followup_frame::FollowupOpKind::Read,
            bound_target: Some("/workspace/README.md".to_string()),
            ordered_entries: vec!["README.md".to_string()],
            ..Default::default()
        }),
        ..empty_snapshot()
    };

    let context = build_active_execution_anchor_context(&snapshot);
    assert!(context.contains("followup_bound_target: /workspace/README.md"));
    assert!(context.contains("followup_ordered_entries: 1:README.md"));
    assert!(session_snapshot_provides_execution_state_anchor(&snapshot));
}

#[test]
fn task_goal_context_uses_structured_payload_only() {
    let task = crate::ClaimedTask {
        task_id: "task-goal".to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: serde_json::json!({
            "goal": {"objective": "verify workspace", "done": ["tests_pass"]}
        })
        .to_string(),
    };

    let context = build_task_goal_context(&task);
    assert!(context.contains("### TASK_GOAL_CONTEXT"));
    assert!(context.contains("verify workspace"));
    assert!(context.contains("tests_pass"));
}

#[test]
fn execution_context_is_projected_to_planner_and_chat_prompts() {
    let bundle = TaskContextBundle {
        raw_sources: TaskContextRawSources::default(),
        planner_view: PlannerContextView::default(),
        execution_view: Some(ExecutionContextView {
            budget_tier: ExecutionContextBudgetTier::Full,
            memory_ctx: empty_prompt_memory_context(),
            runtime_context: "### RUNTIME_CONTEXT\nworkspace_root: /workspace".to_string(),
            goal_context: "### TASK_GOAL_CONTEXT\n{}".to_string(),
            active_task_context: "<none>".to_string(),
            active_execution_anchor_context: "<none>".to_string(),
            session_alias_context: "<none>".to_string(),
            recent_turns_full: "### RECENT_TURNS\nturn".to_string(),
            last_turn_full: "<none>".to_string(),
            recent_execution_anchor: "### EXECUTION_ANCHOR\nstep".to_string(),
            recent_execution_context: "<none>".to_string(),
            image_context: None,
        }),
    };
    let mut chat = String::new();
    let mut resolved = "request".to_string();
    let mut memory = "request".to_string();

    apply_execution_context_to_prompts(&bundle, &mut chat, &mut resolved, &mut memory);

    assert!(chat.contains("### RUNTIME_CONTEXT"));
    assert!(chat.contains("### RECENT_TURNS"));
    assert!(resolved.contains("### TASK_GOAL_CONTEXT"));
    assert!(resolved.contains("### CONTEXT_BUDGET_REPORT"));
    assert!(memory.contains("### RECENT_EXECUTION_CONTEXT"));
}

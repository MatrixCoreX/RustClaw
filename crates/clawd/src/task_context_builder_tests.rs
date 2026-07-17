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
    let recent_execution_anchor = format!("### EXECUTION_ANCHOR\n{}", "x".repeat(1_200));
    let bundle = TaskContextBundle {
        raw_sources: TaskContextRawSources::default(),
        planner_view: PlannerContextView::default(),
        context_source_task_ids: Vec::new(),
        execution_view: Some(ExecutionContextView {
            budget_tier: ExecutionContextBudgetTier::Full,
            memory_ctx: empty_prompt_memory_context(),
            runtime_context: "### RUNTIME_CONTEXT\nworkspace_root: /workspace".to_string(),
            goal_context: "### TASK_GOAL_CONTEXT\n{}".to_string(),
            active_task_context: "### ACTIVE_TASK_CONTEXT\nlast_primary_task_prompt: inspect"
                .to_string(),
            active_execution_anchor_context:
                "### ACTIVE_EXECUTION_ANCHOR\nfollowup_bound_target: /workspace/a.txt".to_string(),
            session_alias_context:
                "### SESSION_ALIAS_BINDINGS\n- alias: report\n  target: /workspace/a.txt"
                    .to_string(),
            recent_turns_full: "### RECENT_TURNS\nturn".to_string(),
            last_turn_full: "<none>".to_string(),
            recent_execution_anchor: recent_execution_anchor.clone(),
            recent_execution_context: "<none>".to_string(),
            compacted_history_context: "### COMPACTED_HISTORY_CONTEXT\n{}".to_string(),
            image_context: None,
        }),
        compaction_records: Vec::new(),
    };
    let mut chat = String::new();
    let mut resolved = "request".to_string();
    let mut memory = "request".to_string();
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");

    let attribution =
        apply_execution_context_to_prompts(&state, &bundle, &mut chat, &mut resolved, &mut memory)
            .expect("render layered context prompts");

    assert!(chat.contains("### RUNTIME_CONTEXT"));
    assert!(chat.contains("### RECENT_TURNS"));
    assert!(resolved.contains("### TASK_GOAL_CONTEXT"));
    assert!(resolved.contains("### CONTEXT_BUDGET_REPORT"));
    assert!(memory.contains("### RECENT_EXECUTION_CONTEXT"));
    assert!(resolved.contains("### COMPACTED_HISTORY_CONTEXT"));
    assert!(memory.contains("### COMPACTED_HISTORY_CONTEXT"));
    assert_eq!(attribution.len(), 5);
    assert_eq!(attribution[0]["prompt_kind"], "runtime_context");
    assert_eq!(attribution[1]["prompt_kind"], "session_aliases");
    assert_eq!(attribution[2]["prompt_kind"], "active_task");
    assert_eq!(attribution[3]["prompt_kind"], "active_execution_anchor");
    assert_eq!(attribution[4]["prompt_kind"], "recent_execution");
    assert!(attribution.iter().all(|item| {
        item["template_char_count"].as_u64().unwrap() <= 2_000
            && item["overhead_char_count"].as_u64().unwrap() <= 1_800
    }));
    assert!(!memory.contains("__RUNTIME_CONTEXT__"));
    assert!(!memory.contains("__SESSION_ALIAS_BINDINGS__"));
    assert!(!memory.contains("__ACTIVE_TASK_CONTEXT__"));
    assert!(!memory.contains("__ACTIVE_EXECUTION_ANCHOR__"));
    assert!(!memory.contains("__RECENT_EXECUTION_CONTEXT__"));
    assert_eq!(memory.matches(&recent_execution_anchor).count(), 1);
}

#[test]
fn context_prompt_dynamic_slots_are_declared_once() {
    for (name, source, placeholder) in [
        (
            "runtime",
            include_str!("../../../prompts/layers/overlays/context_runtime_context.md"),
            "__RUNTIME_CONTEXT__",
        ),
        (
            "session_aliases",
            include_str!("../../../prompts/layers/overlays/context_session_aliases.md"),
            "__SESSION_ALIAS_BINDINGS__",
        ),
        (
            "active_task",
            include_str!("../../../prompts/layers/overlays/context_active_task.md"),
            "__ACTIVE_TASK_CONTEXT__",
        ),
        (
            "active_execution_anchor",
            include_str!("../../../prompts/layers/overlays/context_active_execution_anchor.md"),
            "__ACTIVE_EXECUTION_ANCHOR__",
        ),
        (
            "recent_execution",
            include_str!("../../../prompts/layers/overlays/context_recent_execution.md"),
            "__RECENT_EXECUTION_CONTEXT__",
        ),
        (
            "compaction",
            include_str!("../../../prompts/layers/overlays/context_compaction_prompt.md"),
            "__CONTEXT_SOURCE_BUNDLE__",
        ),
    ] {
        assert_eq!(
            source.matches(placeholder).count(),
            1,
            "{name} must expand dynamic context exactly once"
        );
    }
}

use super::{
    ExecutionContextBudgetTier, ExecutionContextView, PlannerContextView, TaskContextBundle,
    TaskContextRawSources,
};

fn empty_prompt_memory_context() -> crate::memory::service::PromptMemoryContext {
    crate::memory::service::PromptMemoryContext {
        prompt_with_memory: "memory".to_string(),
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
fn summary_emits_transcript_compaction_record_for_light_execution_budget() {
    let bundle = TaskContextBundle {
        raw_sources: TaskContextRawSources::default(),
        planner_view: PlannerContextView::default(),
        route_view: None,
        execution_view: Some(ExecutionContextView {
            budget_tier: ExecutionContextBudgetTier::Light,
            memory_ctx: empty_prompt_memory_context(),
            runtime_context: "runtime".to_string(),
            goal_context: "goal".to_string(),
            active_task_context: "<none>".to_string(),
            active_execution_anchor_context: "<none>".to_string(),
            session_alias_context: "<none>".to_string(),
            recent_turns_full: "<none>".to_string(),
            last_turn_full: "<none>".to_string(),
            recent_execution_anchor: "<none>".to_string(),
            recent_execution_context: "<none>".to_string(),
            image_context: None,
        }),
    };

    let summary = bundle.summary();
    let (_, budget_tail) = summary
        .split_once("context_budget_report=")
        .expect("summary should include context budget report");
    let (budget_json, _) = budget_tail
        .split_once(" transcript_compaction_records=")
        .expect("context budget should precede compaction records");
    let budget: serde_json::Value = serde_json::from_str(budget_json).unwrap();
    let (_, tail) = summary
        .split_once("transcript_compaction_records=")
        .expect("summary should include compaction records field");
    let records: serde_json::Value = serde_json::from_str(tail.trim()).unwrap();

    assert_eq!(
        budget["context_input_inventory"]["inputs"][1]["input_kind"],
        "memory_recent_records"
    );
    assert_eq!(
        budget["context_input_inventory"]["inputs"][1]["status"],
        "attached"
    );
    assert_eq!(
        budget["context_input_inventory"]["inputs"][2]["input_kind"],
        "goal_fields"
    );
    assert_eq!(budget["compaction_triggers"][0], "over_budget");
    assert_eq!(records[0]["summary_kind"], "deterministic_context_budget");
    assert_eq!(records[0]["active_goal_refs"][0], "goal_context");
    assert_eq!(records[0]["source_refs"][0]["ref"], "recent_turns_full");
    assert_eq!(
        records[0]["risk_flags"][1],
        "old_assistant_output_not_instruction"
    );
}

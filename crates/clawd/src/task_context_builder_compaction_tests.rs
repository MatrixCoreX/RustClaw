use super::{
    plan_agent_loop_context_compaction, ExecutionContextBudgetTier, ExecutionContextView,
    PlannerContextView, TaskContextBundle, TaskContextRawSources,
};
use crate::task_context_builder::compaction::apply_context_compaction_with_inputs;

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

fn context_bundle(recent_turns_chars: usize) -> TaskContextBundle {
    TaskContextBundle {
        raw_sources: TaskContextRawSources::default(),
        planner_view: PlannerContextView::default(),
        execution_view: Some(ExecutionContextView {
            budget_tier: ExecutionContextBudgetTier::Full,
            memory_ctx: empty_prompt_memory_context(),
            runtime_context: "runtime".to_string(),
            goal_context: "goal".to_string(),
            active_task_context: "active_task".to_string(),
            active_execution_anchor_context: "active_anchor".to_string(),
            session_alias_context: "<none>".to_string(),
            recent_turns_full: "x".repeat(recent_turns_chars),
            last_turn_full: "last_turn".to_string(),
            recent_execution_anchor: "execution_anchor".to_string(),
            recent_execution_context: "execution_history".to_string(),
            image_context: None,
        }),
        compaction_records: Vec::new(),
    }
}

#[test]
fn transcript_budget_triggers_before_context_provider_truncation() {
    let plan = plan_agent_loop_context_compaction(&context_bundle(13_000))
        .expect("long transcript should trigger compaction");

    assert_eq!(plan.generation, 1);
    assert!(plan.before_char_count > plan.transcript_char_count);
    assert_eq!(plan.threshold_chars, 24_000);
    assert!(plan.trigger_codes.contains(&"transcript_budget_exceeded"));
    assert_eq!(plan.hook_metadata()["generation"], 1);
}

#[test]
fn bounded_context_does_not_trigger_compaction() {
    assert!(plan_agent_loop_context_compaction(&context_bundle(1_000)).is_none());
}

#[test]
fn fifty_turn_fixture_compacts_history_and_retains_active_machine_context() {
    let transcript = (0..52)
        .map(|index| format!("[TURN -{}]\n{}\n[/TURN]", index + 1, "x".repeat(320)))
        .collect::<Vec<_>>()
        .join("\n");
    let mut bundle = context_bundle(0);
    bundle.execution_view.as_mut().unwrap().recent_turns_full = transcript;
    let plan = plan_agent_loop_context_compaction(&bundle).expect("52 turns should compact");

    let record = apply_context_compaction_with_inputs(
        "task-context-compaction",
        &mut bundle,
        &plan,
        empty_prompt_memory_context(),
        "last_turn".to_string(),
    );
    let view = bundle.execution_view.as_ref().unwrap();

    assert_eq!(view.budget_tier, ExecutionContextBudgetTier::Light);
    assert_eq!(view.recent_turns_full, "<none>");
    assert_eq!(view.recent_execution_context, "<none>");
    assert_eq!(view.goal_context, "goal");
    assert_eq!(view.active_task_context, "active_task");
    assert_eq!(view.active_execution_anchor_context, "active_anchor");
    assert_eq!(view.recent_execution_anchor, "execution_anchor");
    assert!(
        record["after_char_count"].as_u64().unwrap()
            < record["before_char_count"].as_u64().unwrap()
    );
    assert_eq!(record["generation"], 1);
    assert_eq!(bundle.compaction_records.len(), 1);
}

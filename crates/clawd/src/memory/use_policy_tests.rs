use super::{
    decide_chat_memory_use_policy, decide_planner_memory_use_policy,
    decide_route_memory_use_policy, decide_skill_memory_use_policy,
    filter_structured_memory_context, ChatMemoryContextHint, MemoryUseDecision, MemoryUseProfile,
    PlannerMemoryContextHint,
};
use crate::memory::retrieval::{MemoryContextMode, RetrievedMemoryItem, StructuredMemoryContext};
use crate::runtime::AppState;
use crate::task_context_builder::{ExecutionContextBudgetTier, RouteContextBudgetTier};

fn item(text: &str) -> RetrievedMemoryItem {
    RetrievedMemoryItem {
        role: Some("assistant".to_string()),
        text: text.to_string(),
        score: 0.9,
        source_label: None,
    }
}

fn full_context() -> StructuredMemoryContext {
    StructuredMemoryContext {
        long_term_summary: Some("legacy summary".to_string()),
        preferences: vec![("response_language".to_string(), "zh-CN".to_string())],
        similar_triggers: vec![item("similar trigger")],
        relevant_facts: vec![item("stable fact")],
        knowledge_docs: vec![item("kb doc")],
        recent_related_events: vec![item("recent event")],
        assistant_results: vec![item("assistant result")],
        unfinished_goals: vec![item("unfinished goal")],
        recalled_recent: vec![("assistant".to_string(), "recent snippet".to_string())],
    }
}

fn empty_snapshot() -> crate::conversation_state::ActiveSessionSnapshot {
    crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    }
}

#[test]
fn route_memory_new_task_omits_assistant_results() {
    let state = AppState::test_default_with_fixture_provider();
    let surface = crate::intent::surface_signals::PromptSurfaceSignals::default();
    let decision = decide_route_memory_use_policy(
        &state,
        RouteContextBudgetTier::Full,
        &surface,
        &empty_snapshot(),
    );
    assert_eq!(decision.profile, MemoryUseProfile::RouteMinimal);

    let filtered = filter_structured_memory_context(full_context(), &decision);
    assert!(filtered.assistant_results.is_empty());
    assert!(filtered.similar_triggers.is_empty());
    assert!(filtered.recent_related_events.is_empty());
    assert!(filtered.unfinished_goals.is_empty());
    assert!(filtered.recalled_recent.is_empty());
    assert!(filtered.long_term_summary.is_none());
    assert_eq!(filtered.preferences.len(), 1);
    assert!(filtered.relevant_facts.is_empty());
    assert!(filtered.knowledge_docs.is_empty());
}

#[test]
fn route_memory_followup_includes_recent_results() {
    let state = AppState::test_default_with_fixture_provider();
    let surface = crate::intent::surface_signals::PromptSurfaceSignals::default();
    let mut snapshot = empty_snapshot();
    snapshot.active_followup_frame = Some(crate::followup_frame::FollowupFrame::default());

    let decision =
        decide_route_memory_use_policy(&state, RouteContextBudgetTier::Full, &surface, &snapshot);
    assert_eq!(decision.profile, MemoryUseProfile::RouteFollowup);

    let filtered = filter_structured_memory_context(full_context(), &decision);
    assert_eq!(filtered.assistant_results.len(), 1);
    assert_eq!(filtered.similar_triggers.len(), 1);
    assert_eq!(filtered.recent_related_events.len(), 1);
    assert_eq!(filtered.unfinished_goals.len(), 1);
    assert_eq!(filtered.recalled_recent.len(), 1);
    assert!(filtered.long_term_summary.is_none());
}

#[test]
fn skill_memory_scoped_policy_omits_conversational_history() {
    let decision = MemoryUseDecision::skill_scoped(1024, "test");
    let filtered = filter_structured_memory_context(full_context(), &decision);
    assert!(filtered.assistant_results.is_empty());
    assert!(filtered.similar_triggers.is_empty());
    assert!(filtered.recent_related_events.is_empty());
    assert!(filtered.unfinished_goals.is_empty());
    assert!(filtered.recalled_recent.is_empty());
    assert!(filtered.long_term_summary.is_none());
    assert_eq!(filtered.preferences.len(), 1);
    assert_eq!(filtered.relevant_facts.len(), 1);
    assert_eq!(filtered.knowledge_docs.len(), 1);
}

#[test]
fn planner_memory_includes_unfinished_goals_and_stable_context() {
    let state = AppState::test_default_with_fixture_provider();
    let decision = decide_planner_memory_use_policy(
        &state,
        ExecutionContextBudgetTier::Full,
        &crate::AskMode::planner_execute_plain(),
        PlannerMemoryContextHint::Default,
    );
    assert_eq!(decision.profile, MemoryUseProfile::PlannerScoped);
    assert!(decision.include_preferences);
    assert!(decision.include_unfinished_goals);
    assert!(decision.include_relevant_facts);
    assert!(decision.include_knowledge_docs);
    assert!(!decision.include_long_term_summary);
    assert!(!decision.include_assistant_results);
    assert!(!decision.include_similar_triggers);
    assert!(!decision.include_recent_related_events);

    let filtered = filter_structured_memory_context(full_context(), &decision);
    assert_eq!(filtered.unfinished_goals.len(), 1);
    assert_eq!(filtered.preferences.len(), 1);
    assert_eq!(filtered.relevant_facts.len(), 1);
    assert_eq!(filtered.knowledge_docs.len(), 1);
    assert!(filtered.long_term_summary.is_none());
    assert!(filtered.assistant_results.is_empty());
    assert!(filtered.similar_triggers.is_empty());
    assert!(filtered.recent_related_events.is_empty());
}

#[test]
fn planner_memory_stable_facts_disabled_keeps_docs_but_omits_facts_and_goals() {
    let state = AppState::test_default_with_fixture_provider();
    let decision = decide_planner_memory_use_policy(
        &state,
        ExecutionContextBudgetTier::Full,
        &crate::AskMode::direct_answer(),
        PlannerMemoryContextHint::StableFactsDisabled,
    );
    assert_eq!(decision.profile, MemoryUseProfile::PlannerScoped);
    assert_eq!(
        decision.reason,
        "standalone_planner_uses_knowledge_docs_without_stable_facts"
    );
    assert!(decision.include_preferences);
    assert!(!decision.include_unfinished_goals);
    assert!(!decision.include_relevant_facts);
    assert!(decision.include_knowledge_docs);
    assert!(!decision.include_long_term_summary);
    assert!(!decision.include_assistant_results);
    assert!(!decision.include_similar_triggers);
    assert!(!decision.include_recent_related_events);

    let filtered = filter_structured_memory_context(full_context(), &decision);
    assert_eq!(filtered.preferences.len(), 1);
    assert!(filtered.unfinished_goals.is_empty());
    assert!(filtered.relevant_facts.is_empty());
    assert_eq!(filtered.knowledge_docs.len(), 1);
    assert!(filtered.long_term_summary.is_none());
    assert!(filtered.assistant_results.is_empty());
    assert!(filtered.similar_triggers.is_empty());
    assert!(filtered.recent_related_events.is_empty());
}

#[test]
fn chat_memory_pure_direct_answer_omits_long_term_and_assistant_results() {
    let state = AppState::test_default_with_fixture_provider();
    let decision = decide_chat_memory_use_policy(
        &state,
        ExecutionContextBudgetTier::Full,
        &crate::AskMode::direct_answer(),
        "",
        false,
        1200,
        ChatMemoryContextHint::Default,
    );
    assert_eq!(decision.profile, MemoryUseProfile::ChatScoped);
    assert!(decision.include_preferences);
    assert!(decision.include_relevant_facts);
    assert!(decision.include_knowledge_docs);
    assert!(!decision.include_long_term_summary);
    assert!(!decision.include_assistant_results);
    assert!(!decision.include_similar_triggers);
    assert!(!decision.include_unfinished_goals);
    assert!(!decision.include_recent_related_events);
    assert!(!decision.include_recent_snippets);

    let filtered = filter_structured_memory_context(full_context(), &decision);
    assert_eq!(filtered.preferences.len(), 1);
    assert_eq!(filtered.relevant_facts.len(), 1);
    assert_eq!(filtered.knowledge_docs.len(), 1);
    assert!(filtered.long_term_summary.is_none());
    assert!(filtered.assistant_results.is_empty());
    assert!(filtered.similar_triggers.is_empty());
    assert!(filtered.unfinished_goals.is_empty());
    assert!(filtered.recent_related_events.is_empty());
    assert!(filtered.recalled_recent.is_empty());
}

#[test]
fn chat_memory_active_session_allows_bounded_recent_context_only() {
    let state = AppState::test_default_with_fixture_provider();
    let decision = decide_chat_memory_use_policy(
        &state,
        ExecutionContextBudgetTier::Full,
        &crate::AskMode::direct_answer(),
        "",
        true,
        1200,
        ChatMemoryContextHint::Default,
    );
    assert_eq!(decision.profile, MemoryUseProfile::ChatScoped);
    assert!(decision.include_recent_related_events);
    assert!(decision.include_recent_snippets);
    assert!(!decision.include_assistant_results);
    assert!(!decision.include_similar_triggers);
    assert!(!decision.include_long_term_summary);

    let filtered = filter_structured_memory_context(full_context(), &decision);
    assert_eq!(filtered.recent_related_events.len(), 1);
    assert_eq!(filtered.recalled_recent.len(), 1);
    assert!(filtered.assistant_results.is_empty());
    assert!(filtered.similar_triggers.is_empty());
    assert!(filtered.long_term_summary.is_none());
}

#[test]
fn chat_memory_standalone_freeform_clarify_loop_context_is_disabled() {
    let state = AppState::test_default_with_fixture_provider();
    let decision = decide_chat_memory_use_policy(
        &state,
        ExecutionContextBudgetTier::Full,
        &crate::AskMode::direct_answer(),
        "standalone_freeform_clarify_loop_context",
        false,
        1200,
        ChatMemoryContextHint::Default,
    );
    assert_eq!(decision.profile, MemoryUseProfile::Disabled);
    assert_eq!(decision.mode, MemoryContextMode::Chat);
    assert!(!decision.include_preferences);
    assert!(!decision.include_relevant_facts);
    assert!(!decision.include_knowledge_docs);
    assert_eq!(
        decision.reason,
        "standalone_freeform_clarify_loop_context_uses_current_request_only"
    );

    let filtered = filter_structured_memory_context(full_context(), &decision);
    assert!(filtered.preferences.is_empty());
    assert!(filtered.relevant_facts.is_empty());
    assert!(filtered.knowledge_docs.is_empty());
    assert!(filtered.recalled_recent.is_empty());
}

#[test]
fn chat_memory_current_request_only_disables_indexed_memory_and_preferences() {
    let state = AppState::test_default_with_fixture_provider();
    let decision = decide_chat_memory_use_policy(
        &state,
        ExecutionContextBudgetTier::Full,
        &crate::AskMode::direct_answer(),
        "",
        false,
        1200,
        ChatMemoryContextHint::CurrentRequestOnly,
    );
    assert_eq!(decision.profile, MemoryUseProfile::Disabled);
    assert_eq!(
        decision.reason,
        "standalone_direct_answer_uses_current_request_only"
    );
    assert!(!decision.include_preferences);
    assert!(!decision.include_relevant_facts);
    assert!(!decision.include_knowledge_docs);

    let filtered = filter_structured_memory_context(full_context(), &decision);
    assert!(filtered.preferences.is_empty());
    assert!(filtered.relevant_facts.is_empty());
    assert!(filtered.knowledge_docs.is_empty());
}

#[test]
fn chat_memory_active_task_context_only_disables_indexed_memory() {
    let state = AppState::test_default_with_fixture_provider();
    let decision = decide_chat_memory_use_policy(
        &state,
        ExecutionContextBudgetTier::Full,
        &crate::AskMode::direct_answer(),
        "",
        true,
        1200,
        ChatMemoryContextHint::ActiveTaskContextOnly,
    );
    assert_eq!(decision.profile, MemoryUseProfile::Disabled);
    assert_eq!(
        decision.reason,
        "active_task_direct_answer_uses_active_task_context_only"
    );
    assert!(!decision.include_preferences);
    assert!(!decision.include_relevant_facts);
    assert!(!decision.include_knowledge_docs);
    assert!(!decision.include_recent_related_events);
    assert!(!decision.include_recent_snippets);

    let filtered = filter_structured_memory_context(full_context(), &decision);
    assert!(filtered.preferences.is_empty());
    assert!(filtered.relevant_facts.is_empty());
    assert!(filtered.knowledge_docs.is_empty());
    assert!(filtered.recent_related_events.is_empty());
    assert!(filtered.recalled_recent.is_empty());
}

#[test]
fn skill_memory_photo_organize_omits_recent_events() {
    let state = AppState::test_default_with_fixture_provider()
        .with_prompt_layers_installed()
        .with_real_skill_registry();
    let decision = decide_skill_memory_use_policy(&state, "photo_sort");
    assert_eq!(decision.profile, MemoryUseProfile::SkillScoped);
    assert_eq!(
        decision.reason,
        "photo_organize_structured_args_stable_memory_only"
    );
    assert!(decision.include_preferences);
    assert!(decision.include_relevant_facts);
    assert!(decision.include_knowledge_docs);
    assert!(!decision.include_long_term_summary);
    assert!(!decision.include_recent_related_events);
    assert!(!decision.include_assistant_results);
    assert!(!decision.include_similar_triggers);
    assert!(!decision.include_unfinished_goals);
    assert!(!decision.include_recent_snippets);

    let filtered = filter_structured_memory_context(full_context(), &decision);
    assert!(filtered.assistant_results.is_empty());
    assert!(filtered.similar_triggers.is_empty());
    assert!(filtered.recent_related_events.is_empty());
    assert!(filtered.unfinished_goals.is_empty());
    assert!(filtered.recalled_recent.is_empty());
    assert!(filtered.long_term_summary.is_none());
    assert_eq!(filtered.preferences.len(), 1);
    assert_eq!(filtered.relevant_facts.len(), 1);
    assert_eq!(filtered.knowledge_docs.len(), 1);
}

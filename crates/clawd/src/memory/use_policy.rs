use crate::intent::surface_signals::PromptSurfaceSignals;
use crate::memory::retrieval::{MemoryContextMode, StructuredMemoryContext};
use crate::task_context_builder::{ExecutionContextBudgetTier, RouteContextBudgetTier};
use crate::AppState;
use claw_core::skill_registry::{SkillMemoryPolicyConfig, SkillMemoryPolicyProfile};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MemoryUseProfile {
    Disabled,
    RouteMinimal,
    RouteFollowup,
    PlannerScoped,
    ChatScoped,
    SkillScoped,
}

impl MemoryUseProfile {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::RouteMinimal => "route_minimal",
            Self::RouteFollowup => "route_followup",
            Self::PlannerScoped => "planner_scoped",
            Self::ChatScoped => "chat_scoped",
            Self::SkillScoped => "skill_scoped",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MemoryUseDecision {
    pub(crate) profile: MemoryUseProfile,
    pub(crate) mode: MemoryContextMode,
    pub(crate) include_preferences: bool,
    pub(crate) include_long_term_summary: bool,
    pub(crate) include_recent_related_events: bool,
    pub(crate) include_assistant_results: bool,
    pub(crate) include_similar_triggers: bool,
    pub(crate) include_unfinished_goals: bool,
    pub(crate) include_relevant_facts: bool,
    pub(crate) include_knowledge_docs: bool,
    pub(crate) include_recent_snippets: bool,
    pub(crate) max_chars: usize,
    pub(crate) reason: String,
}

impl MemoryUseDecision {
    pub(crate) fn disabled(mode: MemoryContextMode, reason: impl Into<String>) -> Self {
        Self {
            profile: MemoryUseProfile::Disabled,
            mode,
            include_preferences: false,
            include_long_term_summary: false,
            include_recent_related_events: false,
            include_assistant_results: false,
            include_similar_triggers: false,
            include_unfinished_goals: false,
            include_relevant_facts: false,
            include_knowledge_docs: false,
            include_recent_snippets: false,
            max_chars: 0,
            reason: reason.into(),
        }
    }

    fn route_minimal(max_chars: usize, reason: impl Into<String>) -> Self {
        Self {
            profile: MemoryUseProfile::RouteMinimal,
            mode: MemoryContextMode::Route,
            include_preferences: true,
            include_long_term_summary: false,
            include_recent_related_events: false,
            include_assistant_results: false,
            include_similar_triggers: false,
            include_unfinished_goals: false,
            include_relevant_facts: true,
            include_knowledge_docs: true,
            include_recent_snippets: false,
            max_chars,
            reason: reason.into(),
        }
    }

    fn route_followup(max_chars: usize, reason: impl Into<String>) -> Self {
        Self {
            profile: MemoryUseProfile::RouteFollowup,
            mode: MemoryContextMode::Route,
            include_preferences: true,
            include_long_term_summary: false,
            include_recent_related_events: true,
            include_assistant_results: true,
            include_similar_triggers: true,
            include_unfinished_goals: true,
            include_relevant_facts: true,
            include_knowledge_docs: true,
            include_recent_snippets: true,
            max_chars,
            reason: reason.into(),
        }
    }

    pub(crate) fn planner_scoped(max_chars: usize, reason: impl Into<String>) -> Self {
        Self {
            profile: MemoryUseProfile::PlannerScoped,
            mode: MemoryContextMode::Planner,
            include_preferences: true,
            include_long_term_summary: false,
            include_recent_related_events: false,
            include_assistant_results: false,
            include_similar_triggers: false,
            include_unfinished_goals: true,
            include_relevant_facts: true,
            include_knowledge_docs: true,
            include_recent_snippets: false,
            max_chars,
            reason: reason.into(),
        }
    }

    pub(crate) fn chat_scoped(
        max_chars: usize,
        include_active_recent_context: bool,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            profile: MemoryUseProfile::ChatScoped,
            mode: MemoryContextMode::Chat,
            include_preferences: true,
            include_long_term_summary: false,
            include_recent_related_events: include_active_recent_context,
            include_assistant_results: false,
            include_similar_triggers: false,
            include_unfinished_goals: false,
            include_relevant_facts: true,
            include_knowledge_docs: true,
            include_recent_snippets: include_active_recent_context,
            max_chars,
            reason: reason.into(),
        }
    }

    pub(crate) fn skill_scoped(max_chars: usize, reason: impl Into<String>) -> Self {
        Self {
            profile: MemoryUseProfile::SkillScoped,
            mode: MemoryContextMode::Skill,
            include_preferences: true,
            include_long_term_summary: false,
            include_recent_related_events: false,
            include_assistant_results: false,
            include_similar_triggers: false,
            include_unfinished_goals: false,
            include_relevant_facts: true,
            include_knowledge_docs: true,
            include_recent_snippets: false,
            max_chars,
            reason: reason.into(),
        }
    }

    pub(crate) fn needs_recent_recall(&self) -> bool {
        self.include_recent_related_events || self.include_recent_snippets
    }

    pub(crate) fn needs_indexed_recall(&self) -> bool {
        self.include_recent_related_events
            || self.include_assistant_results
            || self.include_similar_triggers
            || self.include_unfinished_goals
            || self.include_relevant_facts
            || self.include_knowledge_docs
    }

    pub(crate) fn prompt_header(&self) -> String {
        format!(
            "### MEMORY_USE_POLICY\nprofile: {}\nreason: {}",
            self.profile.as_str(),
            self.reason.trim()
        )
    }
}

pub(crate) fn decide_route_memory_use_policy(
    state: &AppState,
    route_budget: RouteContextBudgetTier,
    surface: &PromptSurfaceSignals,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> MemoryUseDecision {
    if !state.policy.memory.route_memory_enabled
        || matches!(route_budget, RouteContextBudgetTier::None)
    {
        return MemoryUseDecision::disabled(
            MemoryContextMode::Route,
            "route_memory_disabled_or_context_budget_none",
        );
    }

    let full_max = state
        .policy
        .memory
        .route_trigger_budget_chars
        .max(384)
        .min(state.policy.memory.route_memory_max_chars.max(384));
    let anchor_max = state
        .policy
        .memory
        .route_trigger_budget_chars
        .max(384)
        .min(640)
        .min(state.policy.memory.route_memory_max_chars.max(384));
    let has_active_followup_state = session_snapshot.active_followup_frame.is_some()
        || session_snapshot.active_clarify_state.is_some()
        || session_snapshot
            .active_observed_facts
            .as_ref()
            .is_some_and(|facts| !facts.is_empty());

    if has_active_followup_state {
        return MemoryUseDecision::route_followup(
            full_max,
            "active_session_state_requires_recent_memory",
        );
    }

    let structural_locator_request = surface.has_explicit_path_or_url()
        || surface.is_structural_locator_only_reply()
        || surface.has_single_filename_candidate()
        || surface.locator_target_pair.is_some()
        || surface.has_structured_target_refinement();

    if matches!(route_budget, RouteContextBudgetTier::AnchorOnly) || structural_locator_request {
        return MemoryUseDecision::route_minimal(
            anchor_max,
            "current_turn_structured_locator_or_anchor_budget",
        );
    }

    MemoryUseDecision::route_minimal(full_max, "new_route_task_uses_stable_memory_only")
}

pub(crate) fn decide_planner_memory_use_policy(
    state: &AppState,
    budget_tier: ExecutionContextBudgetTier,
    ask_mode: &crate::AskMode,
) -> MemoryUseDecision {
    let prompt_cap = state.policy.memory.prompt_max_chars.max(512);
    let max_chars = match budget_tier {
        ExecutionContextBudgetTier::Full => state
            .policy
            .memory
            .agent_memory_budget_chars
            .max(512)
            .min(prompt_cap),
        ExecutionContextBudgetTier::Light => state
            .policy
            .memory
            .agent_memory_budget_chars
            .max(512)
            .min(768)
            .min(prompt_cap),
    };
    let reason = match ask_mode.first_layer_decision() {
        crate::FirstLayerDecision::PlannerExecute => {
            "planner_execution_uses_goals_preferences_and_stable_facts"
        }
        crate::FirstLayerDecision::DirectAnswer => {
            "direct_answer_keeps_planner_promotion_context_stable"
        }
        crate::FirstLayerDecision::Clarify => "clarify_path_keeps_planner_context_stable",
    };
    MemoryUseDecision::planner_scoped(max_chars, reason)
}

pub(crate) fn decide_chat_memory_use_policy(
    state: &AppState,
    budget_tier: ExecutionContextBudgetTier,
    ask_mode: &crate::AskMode,
    has_active_session_state: bool,
    chat_memory_budget_chars: usize,
) -> MemoryUseDecision {
    let prompt_cap = state.policy.memory.prompt_max_chars.max(384);
    let max_chars = match budget_tier {
        ExecutionContextBudgetTier::Full => chat_memory_budget_chars.max(384).min(prompt_cap),
        ExecutionContextBudgetTier::Light => {
            chat_memory_budget_chars.max(384).min(640).min(prompt_cap)
        }
    };
    let include_active_recent_context =
        has_active_session_state && matches!(budget_tier, ExecutionContextBudgetTier::Full);
    let reason = if include_active_recent_context {
        "chat_with_active_session_state_allows_bounded_recent_context"
    } else {
        match ask_mode.first_layer_decision() {
            crate::FirstLayerDecision::DirectAnswer => {
                "pure_direct_answer_uses_stable_memory_without_long_term_summary"
            }
            crate::FirstLayerDecision::PlannerExecute => {
                "planner_chat_finalization_uses_stable_memory_without_long_term_summary"
            }
            crate::FirstLayerDecision::Clarify => {
                "clarify_chat_path_uses_stable_memory_without_long_term_summary"
            }
        }
    };
    MemoryUseDecision::chat_scoped(max_chars, include_active_recent_context, reason)
}

pub(crate) fn decide_skill_memory_use_policy(
    state: &AppState,
    skill_name: &str,
) -> MemoryUseDecision {
    if !state.policy.memory.skill_memory_enabled {
        return MemoryUseDecision::disabled(MemoryContextMode::Skill, "skill_memory_disabled");
    }
    let default_max_chars = state.policy.memory.skill_memory_max_chars.max(384);
    let canonical = state.resolve_canonical_skill_name(skill_name);
    if let Some(policy) = state
        .get_skills_registry()
        .as_ref()
        .and_then(|registry| registry.memory_policy(&canonical))
    {
        return decision_from_skill_memory_policy(policy, default_max_chars);
    }
    MemoryUseDecision::skill_scoped(
        default_max_chars,
        "skill_args_are_current_turn_source_of_truth",
    )
}

fn decision_from_skill_memory_policy(
    policy: &SkillMemoryPolicyConfig,
    default_max_chars: usize,
) -> MemoryUseDecision {
    let reason = policy
        .reason
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("registry_skill_memory_policy");
    if matches!(policy.profile, SkillMemoryPolicyProfile::Disabled) {
        return MemoryUseDecision::disabled(MemoryContextMode::Skill, reason);
    }
    let mut decision = MemoryUseDecision::skill_scoped(
        policy
            .max_chars
            .unwrap_or(default_max_chars)
            .max(128)
            .min(default_max_chars),
        reason,
    );
    if !policy.include.is_empty() {
        set_all_skill_memory_sources(&mut decision, false);
        for token in &policy.include {
            set_skill_memory_source(&mut decision, token, true);
        }
    }
    for token in &policy.exclude {
        set_skill_memory_source(&mut decision, token, false);
    }
    decision
}

fn set_all_skill_memory_sources(decision: &mut MemoryUseDecision, enabled: bool) {
    decision.include_preferences = enabled;
    decision.include_long_term_summary = enabled;
    decision.include_recent_related_events = enabled;
    decision.include_assistant_results = enabled;
    decision.include_similar_triggers = enabled;
    decision.include_unfinished_goals = enabled;
    decision.include_relevant_facts = enabled;
    decision.include_knowledge_docs = enabled;
    decision.include_recent_snippets = enabled;
}

fn set_skill_memory_source(decision: &mut MemoryUseDecision, token: &str, enabled: bool) {
    match token {
        "preferences" => decision.include_preferences = enabled,
        "long_term_summary" => decision.include_long_term_summary = enabled,
        "recent_related_events" => decision.include_recent_related_events = enabled,
        "assistant_results" => decision.include_assistant_results = enabled,
        "similar_triggers" => decision.include_similar_triggers = enabled,
        "unfinished_goals" => decision.include_unfinished_goals = enabled,
        "relevant_facts" => decision.include_relevant_facts = enabled,
        "knowledge_docs" => decision.include_knowledge_docs = enabled,
        "recent_snippets" => decision.include_recent_snippets = enabled,
        _ => {}
    }
}

pub(crate) fn filter_structured_memory_context(
    mut ctx: StructuredMemoryContext,
    decision: &MemoryUseDecision,
) -> StructuredMemoryContext {
    if !decision.include_preferences {
        ctx.preferences.clear();
    }
    if !decision.include_long_term_summary {
        ctx.long_term_summary = None;
    }
    if !decision.include_recent_related_events {
        ctx.recent_related_events.clear();
    }
    if !decision.include_assistant_results {
        ctx.assistant_results.clear();
    }
    if !decision.include_similar_triggers {
        ctx.similar_triggers.clear();
    }
    if !decision.include_unfinished_goals {
        ctx.unfinished_goals.clear();
    }
    if !decision.include_relevant_facts {
        ctx.relevant_facts.clear();
    }
    if !decision.include_knowledge_docs {
        ctx.knowledge_docs.clear();
    }
    if !decision.include_recent_snippets {
        ctx.recalled_recent.clear();
    }
    ctx
}

#[cfg(test)]
mod tests {
    use super::{
        decide_chat_memory_use_policy, decide_planner_memory_use_policy,
        decide_route_memory_use_policy, decide_skill_memory_use_policy,
        filter_structured_memory_context, MemoryUseDecision, MemoryUseProfile,
    };
    use crate::memory::retrieval::{RetrievedMemoryItem, StructuredMemoryContext};
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
        assert_eq!(filtered.relevant_facts.len(), 1);
        assert_eq!(filtered.knowledge_docs.len(), 1);
    }

    #[test]
    fn route_memory_followup_includes_recent_results() {
        let state = AppState::test_default_with_fixture_provider();
        let surface = crate::intent::surface_signals::PromptSurfaceSignals::default();
        let mut snapshot = empty_snapshot();
        snapshot.active_followup_frame = Some(crate::followup_frame::FollowupFrame::default());

        let decision = decide_route_memory_use_policy(
            &state,
            RouteContextBudgetTier::Full,
            &surface,
            &snapshot,
        );
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
    fn chat_memory_pure_direct_answer_omits_long_term_and_assistant_results() {
        let state = AppState::test_default_with_fixture_provider();
        let decision = decide_chat_memory_use_policy(
            &state,
            ExecutionContextBudgetTier::Full,
            &crate::AskMode::direct_answer(),
            false,
            1200,
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
            true,
            1200,
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
}

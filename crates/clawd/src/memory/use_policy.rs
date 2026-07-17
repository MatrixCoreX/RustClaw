use crate::memory::retrieval::{MemoryContextMode, StructuredMemoryContext};
use crate::task_context_builder::ExecutionContextBudgetTier;
use crate::AppState;
use claw_core::skill_registry::{SkillMemoryPolicyConfig, SkillMemoryPolicyProfile};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MemoryUseProfile {
    Disabled,
    PlannerScoped,
    ChatScoped,
    SkillScoped,
}

impl MemoryUseProfile {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
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

pub(crate) fn decide_planner_memory_use_policy(
    state: &AppState,
    budget_tier: ExecutionContextBudgetTier,
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
    MemoryUseDecision::planner_scoped(
        max_chars,
        "agent_loop_uses_goals_preferences_and_stable_facts",
    )
}

pub(crate) fn decide_chat_memory_use_policy(
    state: &AppState,
    budget_tier: ExecutionContextBudgetTier,
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
        "agent_loop_finalization_uses_stable_memory_without_long_term_summary"
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
#[path = "use_policy_tests.rs"]
mod tests;

use crate::memory::retrieval::{MemoryContextMode, RetrievedMemoryItem, StructuredMemoryContext};
use crate::{AppState, ClaimedTask};

pub(crate) struct PromptMemoryContext {
    pub(crate) prompt_with_memory: String,
    pub(crate) chat_prompt_context: String,
    pub(crate) long_term_summary: Option<String>,
    pub(crate) preferences: Vec<(String, String)>,
    pub(crate) recalled: Vec<(String, String)>,
    pub(crate) similar_triggers: Vec<RetrievedMemoryItem>,
    pub(crate) relevant_facts: Vec<RetrievedMemoryItem>,
    pub(crate) recent_related_events: Vec<RetrievedMemoryItem>,
}

pub(crate) fn prepare_prompt_with_memory(
    state: &AppState,
    task: &ClaimedTask,
    prompt: &str,
    chat_memory_budget_chars: usize,
) -> PromptMemoryContext {
    let structured = recall_structured_memory_context(
        state,
        task.user_key.as_deref(),
        task.user_id,
        task.chat_id,
        prompt,
        state.memory.prompt_recall_limit.max(1),
        true,
        true,
    );
    let mut agent_structured = structured.clone();
    // Keep long-term memory available for logs/refresh, but do not pin it into planner context.
    // Planner should rely on current request + recent execution context first.
    agent_structured.long_term_summary = None;
    let prompt_with_memory = structured_memory_context_block(
        &agent_structured,
        MemoryContextMode::Agent,
        state
            .memory
            .agent_memory_budget_chars
            .max(512)
            .min(state.memory.prompt_max_chars.max(512)),
    );
    let chat_prompt_context = structured_memory_context_block(
        &structured,
        MemoryContextMode::Chat,
        chat_memory_budget_chars
            .max(384)
            .min(state.memory.prompt_max_chars.max(384)),
    );
    PromptMemoryContext {
        chat_prompt_context,
        long_term_summary: structured.long_term_summary.clone(),
        preferences: structured.preferences.clone(),
        recalled: crate::memory::retrieval::legacy_pairs_from_structured(&structured),
        similar_triggers: structured.similar_triggers.clone(),
        relevant_facts: structured.relevant_facts.clone(),
        recent_related_events: structured.recent_related_events.clone(),
        prompt_with_memory,
    }
}

pub(crate) fn recall_structured_memory_context(
    state: &AppState,
    user_key: Option<&str>,
    user_id: i64,
    chat_id: i64,
    anchor_prompt: &str,
    recent_limit: usize,
    include_long_term: bool,
    include_preferences: bool,
) -> StructuredMemoryContext {
    let long_term_summary = if include_long_term && state.memory.long_term_enabled {
        crate::memory::recall_long_term_summary(state, user_key, user_id, chat_id)
            .unwrap_or(None)
            .map(|s| crate::truncate_text(&s, state.memory.long_term_recall_max_chars.max(256)))
    } else {
        None
    };
    let preferences = if include_preferences {
        crate::memory::recall_user_preferences(
            state,
            user_key,
            user_id,
            chat_id,
            state.memory.preference_recall_limit.max(1),
        )
        .unwrap_or_default()
    } else {
        Vec::new()
    };
    let recalled_recent =
        crate::memory::recall_recent_memories(state, user_key, user_id, chat_id, recent_limit)
            .unwrap_or_default();
    let recalled_recent = crate::memory::filter_memories_for_prompt_recall(
        recalled_recent,
        state.memory.prefer_llm_assistant_memory,
    );
    let recalled_recent = if state.memory.recent_relevance_enabled {
        crate::memory::select_relevant_memories_for_prompt(
            recalled_recent,
            anchor_prompt,
            state.memory.recent_relevance_min_score.clamp(0.0, 1.0),
        )
    } else {
        recalled_recent
    };

    let indexed = if state.memory.hybrid_recall_enabled {
        crate::memory::retrieval::retrieve_indexed_memories(
            state,
            user_key,
            user_id,
            chat_id,
            anchor_prompt,
        )
        .unwrap_or_default()
    } else {
        crate::memory::retrieval::IndexedRecall::default()
    };

    StructuredMemoryContext {
        long_term_summary,
        preferences,
        similar_triggers: indexed.similar_triggers,
        relevant_facts: indexed.relevant_facts,
        recent_related_events: indexed.recent_related_events,
        recalled_recent,
    }
}

pub(crate) fn structured_memory_context_block(
    ctx: &StructuredMemoryContext,
    mode: MemoryContextMode,
    max_chars: usize,
) -> String {
    crate::memory::retrieval::build_structured_memory_context_block(ctx, mode, max_chars)
}

pub(crate) async fn maybe_refresh_long_term_summary(
    state: &AppState,
    task: &ClaimedTask,
) -> Result<(), String> {
    crate::maybe_refresh_long_term_summary(state, task).await
}

pub(crate) fn insert_memory(
    state: &AppState,
    user_id: i64,
    chat_id: i64,
    user_key: Option<&str>,
    channel: &str,
    external_chat_id: Option<&str>,
    role: &str,
    content: &str,
    max_chars: usize,
) -> anyhow::Result<()> {
    crate::memory::insert_memory(
        state,
        user_id,
        chat_id,
        user_key,
        channel,
        external_chat_id,
        role,
        content,
        max_chars,
    )
}

pub(crate) fn preferred_response_language(preferences: &[(String, String)]) -> Option<String> {
    crate::preferred_response_language(preferences)
}

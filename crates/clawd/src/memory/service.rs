use crate::{AppState, ClaimedTask};

pub(crate) struct PromptMemoryContext {
    pub(crate) prompt_with_memory: String,
    pub(crate) long_term_summary: Option<String>,
    pub(crate) preferences: Vec<(String, String)>,
    pub(crate) recalled: Vec<(String, String)>,
}

pub(crate) fn prepare_prompt_with_memory(
    state: &AppState,
    task: &ClaimedTask,
    prompt: &str,
) -> PromptMemoryContext {
    let long_term_summary = crate::recall_long_term_summary(state, task.user_id, task.chat_id)
        .unwrap_or(None)
        .map(|s| crate::truncate_text(&s, state.memory.long_term_recall_max_chars.max(256)));
    let recalled = crate::recall_recent_memories(
        state,
        task.user_id,
        task.chat_id,
        state.memory.prompt_recall_limit.max(1),
    )
    .unwrap_or_default();
    let recalled = crate::filter_memories_for_prompt_recall(
        recalled,
        state.memory.prefer_llm_assistant_memory,
    );
    let recalled = if state.memory.recent_relevance_enabled {
        crate::select_relevant_memories_for_prompt(
            recalled,
            prompt,
            state.memory.recent_relevance_min_score.clamp(0.0, 1.0),
        )
    } else {
        recalled
    };
    let preferences = crate::recall_user_preferences(
        state,
        task.user_id,
        task.chat_id,
        state.memory.preference_recall_limit.max(1),
    )
    .unwrap_or_default();
    let prompt_with_memory = crate::build_prompt_with_memory(
        prompt,
        long_term_summary.as_deref(),
        &preferences,
        &recalled,
        state.memory.prompt_max_chars.max(512),
    );
    PromptMemoryContext {
        prompt_with_memory,
        long_term_summary,
        preferences,
        recalled,
    }
}

pub(crate) fn recall_memory_context_parts(
    state: &AppState,
    user_id: i64,
    chat_id: i64,
    anchor_prompt: &str,
    recent_limit: usize,
    include_long_term: bool,
    include_preferences: bool,
) -> (Option<String>, Vec<(String, String)>, Vec<(String, String)>) {
    crate::recall_memory_context_parts(
        state,
        user_id,
        chat_id,
        anchor_prompt,
        recent_limit,
        include_long_term,
        include_preferences,
    )
}

pub(crate) fn memory_context_block(
    long_term_summary: Option<&str>,
    preferences: &[(String, String)],
    memories: &[(String, String)],
    max_chars: usize,
) -> String {
    crate::memory_context_block(long_term_summary, preferences, memories, max_chars)
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
    role: &str,
    content: &str,
    max_chars: usize,
) -> anyhow::Result<()> {
    crate::insert_memory(state, user_id, chat_id, role, content, max_chars)
}

pub(crate) fn preferred_response_language(preferences: &[(String, String)]) -> Option<String> {
    crate::preferred_response_language(preferences)
}

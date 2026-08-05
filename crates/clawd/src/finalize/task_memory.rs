use tracing::warn;

use crate::AppState;

use super::task_payload_helpers::should_skip_ask_memory_pair;

pub(super) fn assistant_memory_source_text(
    answer_text: &str,
    answer_messages: &[String],
) -> String {
    let publishable_messages = answer_messages
        .iter()
        .map(|message| message.trim())
        .filter(|message| !message.is_empty())
        .filter(|message| !crate::finalize::is_execution_summary_message(message))
        .collect::<Vec<_>>();
    if publishable_messages.is_empty() {
        let answer = answer_text.trim();
        if crate::finalize::is_execution_summary_message(answer) {
            String::new()
        } else {
            answer.to_string()
        }
    } else {
        publishable_messages.join("\n")
    }
}

pub(super) fn insert_ask_memory_pair(
    state: &AppState,
    task: &crate::ClaimedTask,
    prompt: &str,
    answer_text: &str,
    answer_messages: &[String],
    is_llm_reply: bool,
    agent_display_name_hint: &str,
    force_consolidation: bool,
) {
    if crate::memory::settings::task_memory_generation_enabled(state, task) {
        let _ = crate::memory::upsert_user_preferences_from_route_hint(
            state,
            task.user_id,
            task.chat_id,
            task.user_key.as_deref(),
            agent_display_name_hint,
        );
    }
    if should_skip_ask_memory_pair(state, answer_text, answer_messages) {
        return;
    }
    let assistant_source_text = assistant_memory_source_text(answer_text, answer_messages);
    let assistant_memory_text = if assistant_source_text.trim().is_empty() {
        None
    } else if is_llm_reply && state.policy.memory.mark_llm_reply_in_short_term {
        Some(format!(
            "{}{}",
            crate::memory::LLM_SHORT_TERM_MEMORY_PREFIX,
            assistant_source_text
        ))
    } else {
        Some(assistant_source_text)
    };
    if let Err(error) = crate::memory::jobs::persist_turn_and_enqueue(
        state,
        task,
        prompt,
        assistant_memory_text.as_deref(),
        force_consolidation,
    ) {
        warn!(task_id = task.task_id, error = %error, "memory durable jobs enqueue failed");
    }
}

fn build_unfinished_goal_memory_text(prompt: &str, blocker: &str) -> String {
    serde_json::json!({
        "schema": "agent.memory.unfinished_goal.v1",
        "message_key": "memory.unfinished_goal",
        "user_request": prompt.trim(),
        "blocker": blocker.trim(),
    })
    .to_string()
}

pub(super) fn insert_unfinished_goal_memory(
    state: &AppState,
    task: &crate::ClaimedTask,
    prompt: &str,
    blocker: &str,
) {
    if !crate::memory::settings::task_memory_generation_enabled(state, task) {
        return;
    }
    let text = build_unfinished_goal_memory_text(prompt, blocker);
    let _ = crate::memory::service::insert_memory_with_kind(
        state,
        task.user_id,
        task.chat_id,
        task.user_key.as_deref(),
        &task.channel,
        task.external_chat_id.as_deref(),
        crate::memory::MEMORY_ROLE_SYSTEM,
        &text,
        state.policy.memory.item_max_chars.max(256),
        crate::memory::MemoryWriteKind::UnfinishedGoal,
    );
}

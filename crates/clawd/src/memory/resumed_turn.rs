//! Persist recovered ask turns in the transaction which commits their result.
//! Preparation only reads policy; persistence neither invokes a model nor
//! replays any task action. Losing the terminal compare-and-swap writes nothing.
use rusqlite::Connection;
use serde_json::Value;

use crate::{AppState, ClaimedTask};

pub(crate) struct PreparedTurn {
    task: ClaimedTask,
    prompt: String,
    assistant: Option<String>,
    settings: Option<super::settings::MemoryEffectiveSettings>,
}

pub(crate) fn prepare(
    state: &AppState,
    task: &ClaimedTask,
    status: &str,
    result: &Value,
) -> anyhow::Result<Option<PreparedTurn>> {
    if status != "succeeded" || task.kind != "ask" {
        return Ok(None);
    }
    // Match prepare_ask_input: the opaque original text is the memory input.
    // Do not substitute the checkpoint's machine execution envelope.
    let payload: Value = serde_json::from_str(&task.payload_json)?;
    let prompt = payload
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let Some(answer) = result.get("text").and_then(Value::as_str) else {
        return Ok(None);
    };
    if prompt.trim().is_empty() || answer.trim().is_empty() {
        return Ok(None);
    }
    let messages: Vec<String> = result
        .get("messages")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    if crate::finalize::should_skip_ask_memory_pair(state, answer, &messages) {
        return Ok(None);
    }
    let source = crate::finalize::assistant_memory_source_text(answer, &messages);
    let assistant = (!source.trim().is_empty()).then(|| {
        if state.policy.memory.mark_llm_reply_in_short_term {
            format!("{}{source}", super::LLM_SHORT_TERM_MEMORY_PREFIX)
        } else {
            source
        }
    });
    // Resolve settings before the caller opens its write transaction.
    let settings = super::settings::resolve_task_memory_settings(state, task)?;
    Ok(Some(PreparedTurn {
        task: task.clone(),
        prompt: prompt.to_owned(),
        assistant,
        settings,
    }))
}

impl PreparedTurn {
    pub(crate) fn persist(self, state: &AppState, db: &Connection) -> anyhow::Result<()> {
        super::jobs::persist_turn_and_enqueue_in_connection(
            state,
            db,
            &self.task,
            &self.prompt,
            self.assistant.as_deref(),
            false,
            self.settings.as_ref(),
        )?;
        Ok(())
    }
}

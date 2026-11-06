use tracing::warn;

use super::job_completion::{mark_committed, GenerationLease};
use super::{apply, build_memory_intent_llm_prompt, effective_user_key, intent, settings};
use crate::{AppState, ClaimedTask};

#[cfg(test)]
pub(crate) async fn maybe_extract_memory_intent_with_llm(
    state: &AppState,
    task: &ClaimedTask,
    content: &str,
) -> anyhow::Result<()> {
    extract(state, task, content, None).await
}

pub(super) async fn extract_for_job(
    state: &AppState,
    task: &ClaimedTask,
    content: &str,
    generation: GenerationLease<'_>,
) -> anyhow::Result<()> {
    extract(state, task, content, Some(generation)).await
}

async fn extract(
    state: &AppState,
    task: &ClaimedTask,
    content: &str,
    generation: Option<GenerationLease<'_>>,
) -> anyhow::Result<()> {
    let cfg = &state.policy.memory;
    if !cfg.enable_preference_extraction
        || !cfg.llm_preference_fallback_enabled
        || !settings::task_memory_generation_enabled(state, task)
    {
        return Ok(());
    }
    let trimmed = content.trim();
    if trimmed.chars().count() < cfg.write_min_chars.max(8) {
        return Ok(());
    }
    let prompt_text = crate::utf8_safe_prefix(trimmed, cfg.llm_preference_max_chars.max(128));
    let source_ref = format!("task:{}:user", task.task_id);
    let prompt = build_memory_intent_llm_prompt(prompt_text, &source_ref);
    let raw = match super::generation_response::run(
        state,
        task,
        &prompt,
        "memory_intent_extract",
        generation,
    )
    .await
    {
        Ok(raw) => raw,
        Err(error) => {
            warn!(task_id = task.task_id, error = %error, "memory_intent_extraction_failed");
            return if generation.is_some() {
                Err(error)
            } else {
                Ok(())
            };
        }
    };
    let intent = match intent::parse_memory_intent_schema(&raw) {
        Ok(intent) => intent,
        Err(error) => {
            warn!(task_id = task.task_id, error = %error, "memory_intent_schema_validation_failed");
            return Ok(());
        }
    };
    let validation =
        intent::validate_memory_intent_actions(intent, cfg.llm_preference_min_confidence);
    if !validation.rejected.is_empty() {
        warn!(
            task_id = task.task_id,
            rejected_count = validation.rejected.len(),
            "memory_intent_actions_rejected"
        );
    }
    if validation.accepted.is_empty() {
        return Ok(());
    }
    let user_key = effective_user_key(task.user_key.as_deref(), task.user_id, task.chat_id);
    let db = state.core.db.get()?;
    let tx = db.unchecked_transaction()?;
    let stats = apply::apply_memory_actions(
        state,
        &tx,
        task.user_id,
        task.chat_id,
        &user_key,
        &validation.accepted,
        &crate::now_ts(),
        crate::now_ts_u64() as i64,
    )?;
    if let Some(lease) = generation {
        mark_committed(&tx, lease)?;
    }
    tx.commit()?;
    tracing::info!(
        task_id = task.task_id,
        upserted_preferences = stats.upserted_preferences,
        deleted_preferences = stats.deleted_preferences,
        marked_safety_signals = stats.marked_safety_signals,
        ignored_actions = stats.ignored_actions,
        "memory_intent_applied"
    );
    Ok(())
}

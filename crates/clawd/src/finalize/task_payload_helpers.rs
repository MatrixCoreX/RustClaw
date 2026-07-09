use serde_json::{json, Value};
use std::path::{Path, PathBuf};

use crate::AppState;

pub(super) fn ask_result_payload(
    answer_text: &str,
    answer_messages: &[String],
    journal: Option<&crate::task_journal::TaskJournal>,
) -> Value {
    let visible_answer_text = crate::visible_text::sanitize_user_visible_text(answer_text);
    let visible_answer_messages = answer_messages
        .iter()
        .map(|message| crate::visible_text::sanitize_user_visible_text(message))
        .collect::<Vec<_>>();
    let base_result = if visible_answer_messages.is_empty() {
        json!({ "text": visible_answer_text })
    } else {
        json!({ "text": visible_answer_text, "messages": visible_answer_messages })
    };
    match journal {
        Some(journal) => journal.attach_to_result(base_result),
        None => base_result,
    }
}

pub(super) fn should_skip_ask_memory_pair(
    state: &AppState,
    answer_text: &str,
    answer_messages: &[String],
) -> bool {
    if crate::fallback::is_known_clarify_fallback_text(state, answer_text) {
        return true;
    }
    answer_messages
        .iter()
        .filter(|message| !crate::finalize::is_execution_summary_message(message))
        .any(|message| crate::fallback::is_known_clarify_fallback_text(state, message))
}

pub(super) fn non_failure_final_status(
    semantic_clarify: bool,
) -> crate::task_journal::TaskJournalFinalStatus {
    if semantic_clarify {
        crate::task_journal::TaskJournalFinalStatus::Clarify
    } else {
        crate::task_journal::TaskJournalFinalStatus::Success
    }
}

pub(super) fn answer_verifier_forces_task_failure(
    semantic_clarify: bool,
    journal: &crate::task_journal::TaskJournal,
) -> bool {
    !semantic_clarify
        && journal
            .answer_verifier_summary
            .as_ref()
            .is_some_and(|summary| summary.high_confidence_retry_gap())
}

pub(super) fn answer_verifier_should_force_task_failure(
    enforce_required: bool,
    semantic_clarify: bool,
    journal: &crate::task_journal::TaskJournal,
) -> bool {
    enforce_required && answer_verifier_forces_task_failure(semantic_clarify, journal)
}

pub(super) fn normalize_existing_file_delivery_token_answer(
    state: &AppState,
    answer_text: &str,
) -> Option<String> {
    let trimmed = answer_text.trim();
    if trimmed.is_empty() || trimmed.lines().count() != 1 {
        return None;
    }
    let (kind, payload) = crate::finalize::parse_delivery_file_token(trimmed)?;
    let payload = payload.trim();
    if payload.is_empty()
        || payload.contains('\n')
        || payload.contains('\r')
        || payload.contains("{{")
        || payload.contains("}}")
    {
        return None;
    }
    let path = Path::new(payload);
    let candidate: PathBuf = if path.is_absolute() {
        path.to_path_buf()
    } else {
        state.skill_rt.workspace_root.join(path)
    };
    if !candidate.is_file() {
        return None;
    }
    let resolved = candidate.canonicalize().unwrap_or(candidate);
    Some(format!("{}{}", kind.canonical_prefix(), resolved.display()))
}

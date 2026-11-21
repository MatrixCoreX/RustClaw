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
    _state: &AppState,
    answer_text: &str,
    answer_messages: &[String],
) -> bool {
    if crate::fallback::is_machine_fallback_payload(answer_text) {
        return true;
    }
    answer_messages
        .iter()
        .filter(|message| !crate::finalize::is_execution_summary_message(message))
        .any(|message| crate::fallback::is_machine_fallback_payload(message))
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

pub(super) fn child_clarification_failure_payload(
    payload: &Value,
    semantic_clarify: bool,
) -> Option<Value> {
    // A child cannot complete its delegated result by asking the end user a
    // question. Return the need for input to its parent as an unfinished result;
    // ordinary user-facing clarification retains its existing success semantics.
    (semantic_clarify && crate::repo::child_tasks::is_child_subagent_payload(payload)).then(|| {
        json!({
            "schema_version": 1,
            "owner_layer": "child_task_result_contract",
            "status": "needs_parent_input",
            "error_code": "child_task_needs_parent_input",
            "message_key": "child_task.needs_parent_input",
            "reason_code": "child_task_needs_parent_input",
            "status_code": "child_task_needs_parent_input",
            "retryable": false,
            "side_effect_applied": false,
            "recovery_action": "parent_resolve_child_input",
        })
    })
}

pub(super) fn answer_verifier_forces_task_failure(
    semantic_clarify: bool,
    journal: &crate::task_journal::TaskJournal,
) -> bool {
    let rejected = match journal.answer_verification.as_ref() {
        Some(record) => {
            record.status == crate::answer_verifier::AnswerVerificationStatus::Failed
                && journal
                    .final_answer
                    .as_deref()
                    .is_some_and(|answer| record.matches(answer))
        }
        None => journal
            .answer_verifier_summary
            .as_ref()
            .is_some_and(|summary| !summary.pass && summary.confidence >= 0.55),
    };
    !semantic_clarify && rejected
}

pub(super) fn answer_verifier_should_force_task_failure(
    enforce_required: bool,
    semantic_clarify: bool,
    journal: &crate::task_journal::TaskJournal,
) -> bool {
    enforce_required && answer_verifier_forces_task_failure(semantic_clarify, journal)
}

pub(crate) fn enforce_terminal_ask_reply_status(
    enforce_required: bool,
    payload_json: &str,
    reply: &mut crate::AskReply,
) {
    let Some(journal) = reply.task_journal.as_ref() else {
        return;
    };
    if reply.should_fail_task
        || crate::task_lifecycle::has_matching_nonterminal_checkpoint(
            journal.task_lifecycle.as_ref(),
            journal.task_checkpoint.as_ref(),
        )
    {
        return;
    }
    let semantic_clarify =
        journal.final_status == Some(crate::task_journal::TaskJournalFinalStatus::Clarify);
    let payload: Value = serde_json::from_str(payload_json).unwrap_or(Value::Null);
    let failure = child_clarification_failure_payload(&payload, semantic_clarify)
        .map(|value| value.to_string())
        .or_else(|| {
            crate::repo::child_tasks::child_result_contract_failure(&payload, &reply.text)
                .map(|value| value.to_string())
        })
        .or_else(|| {
            answer_verifier_should_force_task_failure(enforce_required, semantic_clarify, journal)
                .then(|| {
                    journal
                        .answer_verifier_summary
                        .as_ref()
                        .map(|summary| summary.required_evidence_failure_payload_text())
                        .unwrap_or_else(|| {
                            json!({
                                "schema_version": 1,
                                "message_key": "answer_verifier_required_evidence_block",
                                "reason_code": "answer_verifier_required_evidence_block",
                                "status_code": "answer_verifier_required_evidence_block",
                                "failure_attribution": "answer_verifier_gap",
                                "retryable": false,
                            })
                            .to_string()
                        })
                })
        });
    if let Some(failure) = failure {
        reply.should_fail_task = true;
        reply.error_text = Some(failure.clone());
        if let Some(journal) = reply.task_journal.as_mut() {
            journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Failure);
            journal.record_final_failure_attribution_from_error(&failure);
        }
    }
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

#[cfg(test)]
#[path = "task_rejection_status_tests.rs"]
mod rejection_status_tests;

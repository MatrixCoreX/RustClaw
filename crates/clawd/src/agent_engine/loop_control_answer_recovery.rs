use super::*;

#[path = "loop_control_answer_recovery/terminal_format.rs"]
mod terminal_format;

pub(super) use terminal_format::prefer_terminal_model_answer_for_verifier_candidate;
use terminal_format::terminal_model_output_format_gap_satisfies_contract;

pub(super) fn answer_verifier_retry_summary<'a>(
    reply: &'a AskReply,
    route_result: Option<&crate::answer_verifier::AnswerContract>,
) -> Option<&'a crate::task_journal::TaskJournalAnswerVerifierSummary> {
    if reply_final_status_is_clarify(reply) {
        return None;
    }
    let summary = reply
        .task_journal
        .as_ref()
        .and_then(|journal| journal.answer_verifier_summary.as_ref())?;
    if reply.should_fail_task && !reply_failure_is_recoverable_answer_verifier_gap(reply, summary) {
        return None;
    }
    if answer_verifier_gap_is_structurally_satisfied(reply, route_result) {
        return None;
    }
    summary.high_confidence_retry_gap().then_some(summary)
}

fn reply_failure_is_recoverable_answer_verifier_gap(
    reply: &AskReply,
    summary: &crate::task_journal::TaskJournalAnswerVerifierSummary,
) -> bool {
    if !summary.high_confidence_retry_gap() {
        return false;
    }
    let Some(journal) = reply.task_journal.as_ref() else {
        return false;
    };
    if matches!(
        journal.final_status,
        Some(
            crate::task_journal::TaskJournalFinalStatus::Clarify
                | crate::task_journal::TaskJournalFinalStatus::ResumeFailure
        )
    ) {
        return false;
    }
    matches!(
        journal.final_failure_attribution.as_deref(),
        None | Some("answer_verifier_gap") | Some("contract_gap")
    )
}

pub(super) fn suppress_answer_verifier_retry_if_structurally_satisfied(
    reply: &mut AskReply,
    route_result: Option<&crate::answer_verifier::AnswerContract>,
) -> bool {
    if !answer_verifier_gap_is_structurally_satisfied(reply, route_result) {
        return false;
    }
    let Some(journal) = reply.task_journal.as_mut() else {
        return false;
    };
    let Some(summary) = journal.answer_verifier_summary.as_ref() else {
        return false;
    };
    if !summary.high_confidence_retry_gap() {
        return false;
    }
    info!(
        missing_evidence_fields = ?summary.missing_evidence_fields,
        "answer_verifier_retry_suppressed_structural_satisfaction"
    );
    journal.answer_verifier_summary = None;
    true
}

fn answer_verifier_gap_is_structurally_satisfied(
    reply: &AskReply,
    route_result: Option<&crate::answer_verifier::AnswerContract>,
) -> bool {
    let Some(summary) = reply
        .task_journal
        .as_ref()
        .and_then(|journal| journal.answer_verifier_summary.as_ref())
    else {
        return false;
    };
    if !summary.high_confidence_retry_gap() {
        return false;
    }
    let Some(route) = route_result else {
        return false;
    };
    if terminal_model_output_format_gap_satisfies_contract(reply, route) {
        return true;
    }
    let (Some(journal), Some(answer)) = (
        reply.task_journal.as_ref(),
        final_user_answer_candidate(reply),
    ) else {
        return false;
    };
    let answer_contract = crate::answer_verifier::AnswerContract::new(
        &route.request_text,
        route.output_contract.clone(),
    );
    crate::answer_verifier::structurally_satisfies_answer_contract(
        &answer_contract,
        journal,
        answer,
    )
}

pub(super) fn final_user_answer_candidate(reply: &AskReply) -> Option<&str> {
    reply
        .messages
        .iter()
        .rev()
        .map(String::as_str)
        .find(|message| {
            let trimmed = message.trim();
            !trimmed.is_empty() && !crate::finalize::is_execution_summary_message(trimmed)
        })
        .or_else(|| {
            let trimmed = reply.text.trim();
            (!trimmed.is_empty()).then_some(trimmed)
        })
}

pub(super) fn mark_reply_failed_after_answer_verifier_exhausted(
    _user_text: &str,
    reply: &mut AskReply,
    verifier: &crate::task_journal::TaskJournalAnswerVerifierSummary,
) {
    let control_payload = verifier.required_evidence_failure_payload_text();
    let mut messages = reply
        .messages
        .iter()
        .filter(|message| crate::finalize::is_execution_summary_message(message))
        .cloned()
        .collect::<Vec<_>>();
    messages.push(control_payload.clone());
    if let Some(journal) = reply.task_journal.as_mut() {
        journal.record_final_answer(&control_payload);
        journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Failure);
        journal.record_final_failure_attribution_from_error(&control_payload);
    }
    reply.text = control_payload.clone();
    reply.messages = messages;
    reply.should_fail_task = true;
    reply.error_text = Some(control_payload);
}

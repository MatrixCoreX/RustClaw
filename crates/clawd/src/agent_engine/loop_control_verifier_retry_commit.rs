use crate::{
    answer_verifier::AnswerVerifierOut, task_journal::TaskJournalAnswerVerifierSummary, AskReply,
};

pub(super) fn retry_verifier_introduced_execution_gap_after_synthesis_only_retry(
    previous: &TaskJournalAnswerVerifierSummary,
    retry: &AnswerVerifierOut,
) -> bool {
    let previous_is_synthesis_only = !previous.missing_evidence_fields.is_empty()
        && previous
            .missing_evidence_fields
            .iter()
            .all(|field| matches!(field.as_str(), "output_format" | "unsupported_claims"));
    if !previous_is_synthesis_only {
        return false;
    }

    let previous_had_execution_gap = previous
        .missing_evidence_fields
        .iter()
        .any(|field| field == "requested_result" || field.starts_with("required_dispatch:"));
    if previous_had_execution_gap {
        return false;
    }

    let retry_has_required_dispatch = retry
        .missing_evidence_fields
        .iter()
        .any(|field| field.starts_with("required_dispatch:"));
    retry_has_required_dispatch
        && retry
            .missing_evidence_fields
            .iter()
            .all(|field| field == "requested_result" || field.starts_with("required_dispatch:"))
}

pub(super) fn verifier_retry_answer_has_required_machine_evidence(
    reply: &AskReply,
    answer: &str,
) -> bool {
    crate::finalize::answer_verifier_retry_answer_has_required_machine_evidence(
        reply.task_journal.as_ref(),
        answer,
    )
}

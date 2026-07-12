use crate::AskReply;

pub(super) fn verifier_retry_answer_has_required_machine_evidence(
    reply: &AskReply,
    answer: &str,
) -> bool {
    crate::finalize::answer_verifier_retry_answer_has_required_machine_evidence(
        reply.task_journal.as_ref(),
        answer,
    )
}

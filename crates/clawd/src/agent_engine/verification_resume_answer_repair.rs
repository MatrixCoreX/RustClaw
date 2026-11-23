//! Preserve the verified execution while repairing a rejected resumed answer.
use super::*;

fn answer_only(summary: &crate::task_journal::TaskJournalAnswerVerifierSummary) -> bool {
    summary.is_answer_only_retry_gap()
}

pub(super) async fn try_repair(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut LoopState,
    contract: &crate::answer_verifier::AnswerContract,
    mut journal: TaskJournal,
    candidate: &str,
) -> Option<AskReply> {
    let summary = journal
        .answer_verifier_summary
        .clone()
        .filter(answer_only)?;
    let available = take_answer_repair_allowance(loop_state);
    journal.answer_verification_checkpoint = Some(std::sync::Arc::new(
        verification_checkpoint_payload(task, loop_state),
    ));
    let mut reply = AskReply::non_llm(candidate.into()).with_task_journal(journal);
    if available
        && super::super::loop_control::try_bounded_answer_verifier_synthesis_retry(
            state, task, user_text, contract, &summary, &mut reply,
        )
        .await
    {
        return Some(reply);
    }
    let journal = reply
        .task_journal
        .as_mut()
        .expect("repair retains its journal");
    if journal.answer_verification.as_ref().is_some_and(|record| {
        record.status == crate::answer_verifier::AnswerVerificationStatus::Unavailable
    }) {
        crate::answer_verifier::checkpoint_unavailable_verification(state, task, journal);
        journal.record_final_answer("");
        reply.text.clear();
        reply.messages.clear();
        return Some(reply);
    }
    let failure = journal.answer_verifier_summary.clone().unwrap_or(summary);
    super::super::loop_control::mark_reply_failed_after_answer_verifier_exhausted(
        user_text, &mut reply, &failure,
    );
    Some(reply)
}

pub(super) fn record_rejection(loop_state: &mut LoopState, journal: &TaskJournal, candidate: &str) {
    let observation = json!({
        "schema_version":1,"observation_kind":"answer_verification_resume_rejected",
        "status":"failed","reason_code":"verification_resume_rejected",
        "answer_verification":journal.answer_verification,
        "missing_evidence_fields":journal.answer_verifier_summary.as_ref()
            .map(|summary| &summary.missing_evidence_fields),
        "model_feedback":{"source":"answer_verifier","trust":"untrusted_model_output",
            "candidate_answer":candidate,
            "verifier_issue":journal.answer_verifier_summary.as_ref().map(|summary| json!({
                "missing_evidence_fields":summary.missing_evidence_fields,
                "answer_incomplete_reason":summary.answer_incomplete_reason,
                "retry_instruction":summary.retry_instruction,
            }))},
        "next_action":"repair_from_current_evidence","terminal_response_allowed":false,
    });
    loop_state.last_output = Some(observation.to_string());
    loop_state.task_observations.push(observation);
    loop_state.has_recoverable_failure_context = true;
    loop_state.delivery_messages.clear();
    loop_state.last_user_visible_respond = None;
    loop_state.last_publishable_synthesis_output = None;
    loop_state.last_capability_synthesis_output = None;
}

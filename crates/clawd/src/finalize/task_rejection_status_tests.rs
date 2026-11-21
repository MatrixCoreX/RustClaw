use super::*;

#[test]
fn terminal_verifier_rejection_is_failure_without_retry_permission() {
    let mut journal =
        crate::task_journal::TaskJournal::for_task("reject-no-retry", "ask", "inspect");
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        should_retry: false,
        confidence: 0.99,
        missing_evidence_fields: vec!["requested_result".into()],
        ..Default::default()
    });
    assert!(answer_verifier_should_force_task_failure(
        true, false, &journal
    ));
    assert!(!answer_verifier_should_force_task_failure(
        true, true, &journal
    ));
    assert!(!journal
        .answer_verifier_summary
        .as_ref()
        .unwrap()
        .high_confidence_retry_gap());
}

#[test]
fn matching_failed_candidate_cannot_succeed_but_stale_rejection_does_not_block_new_answer() {
    let mut journal =
        crate::task_journal::TaskJournal::for_task("reject-candidate", "ask", "inspect");
    let verdict = crate::answer_verifier::AnswerVerifierOut {
        pass: false,
        should_retry: false,
        confidence: 0.99,
        ..Default::default()
    };
    journal.record_final_answer("candidate-a");
    journal.record_answer_verification(
        crate::answer_verifier::AnswerVerificationOutcome::Failed(verdict),
        "candidate-a",
    );
    assert!(answer_verifier_should_force_task_failure(
        true, false, &journal
    ));
    journal.record_final_answer("candidate-b");
    assert!(!answer_verifier_should_force_task_failure(
        true, false, &journal
    ));
}

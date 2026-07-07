#[test]
fn answer_verifier_recovery_terminal_marker_skips_finalize_reverification() {
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-1", "ask", "summarize evidence");
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
    journal.record_final_stop_signal(
        crate::task_journal::ANSWER_VERIFIER_RECOVERED_TERMINAL_STOP_SIGNAL,
    );
    journal.answer_verifier_summary = None;

    assert!(super::super::answer_verifier_recovery_already_terminal(
        &journal
    ));

    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["content_excerpt".to_string()],
        answer_incomplete_reason: "still checking".to_string(),
        should_retry: true,
        retry_instruction: "retry".to_string(),
        confidence: 0.9,
    });

    assert!(!super::super::answer_verifier_recovery_already_terminal(
        &journal
    ));
}

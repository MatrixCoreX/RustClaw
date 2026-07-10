use super::*;

#[test]
fn non_failure_final_status_preserves_clarify_semantics() {
    assert_eq!(
        non_failure_final_status(false),
        crate::task_journal::TaskJournalFinalStatus::Success
    );
    assert_eq!(
        non_failure_final_status(true),
        crate::task_journal::TaskJournalFinalStatus::Clarify
    );
}

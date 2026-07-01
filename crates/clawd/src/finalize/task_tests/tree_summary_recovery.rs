use super::super::task_tree_summary_recovery::deterministic_tree_summary_rows_failure_recovery;

#[test]
fn tree_summary_recovery_uses_extra_not_text_payload() {
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-tree-summary-extra", "ask", "");
    journal.record_final_stop_signal("synthesize_answer_failed");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "system_basic",
            r#"{"extra":{"action":"tree_summary","summary_rows":[{"kind":"dir","name":"docs","file_count":2,"truncated":false}]},"text":"{\"action\":\"tree_summary\",\"summary_rows\":[{\"kind\":\"dir\",\"name\":\"must_not_parse_text\",\"file_count\":99}]}"}"#,
        ));

    let recovered =
        deterministic_tree_summary_rows_failure_recovery(&journal).expect("tree summary recovery");

    assert_eq!(recovered, "name=docs file_count=2 truncated=false");
}

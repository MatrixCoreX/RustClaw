use super::{apply_requested_machine_kv_summary_to_final_answer, route_result};

#[test]
fn missing_path_machine_summary_preserves_stable_error_code() {
    let prompt = "Return path, exists, and error_code for missing.md.";
    let route = route_result();
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-missing-path-machine-kv", "ask", prompt);
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            r#"{"action":"path_batch_facts","count":1,"facts":[{"error":"not found","error_code":"path_not_found","exists":false,"kind":"missing","path":"missing.md"}],"include_missing":true}"#,
        ));
    let mut answer_text = "exists=false\npath=missing.md".to_string();
    let mut answer_messages = vec![answer_text.clone()];

    assert!(apply_requested_machine_kv_summary_to_final_answer(
        prompt,
        &route,
        &mut journal,
        &mut answer_text,
        &mut answer_messages,
    ));

    assert!(answer_text.contains("exists=false"));
    assert!(answer_text.contains("path=missing.md"));
    assert!(answer_text.contains("kind=missing"));
    assert!(answer_text.contains("error_code=path_not_found"));
    assert!(answer_text.contains("source_action=path_batch_facts"));
    assert_eq!(answer_messages, vec![answer_text.clone()]);
    assert_eq!(journal.final_answer.as_deref(), Some(answer_text.as_str()));
}

use super::answer_verifier_retry_answer_has_required_machine_evidence;

#[test]
fn answer_verifier_retry_machine_evidence_rejects_local_code_json_without_journal() {
    assert!(!answer_verifier_retry_answer_has_required_machine_evidence(
        None,
        r#"{"changed_files":["calc_core.py"],"test_command":"python3 test_calc_core.py","test_status":"pass"}"#,
    ));
}

#[test]
fn answer_verifier_retry_machine_evidence_rejects_local_code_json_after_terminal_respond_only() {
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-local-code-retry-respond-only",
        "ask",
        "request",
    );
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "respond",
            r#"{"message_key":"clawd.msg.fallback.verify_rejected","reason_code":"verify_rejected"}"#,
        ));

    assert!(!answer_verifier_retry_answer_has_required_machine_evidence(
        Some(&journal),
        r#"{"changed_files":["calc_core.py"],"test_command":"python3 test_calc_core.py","test_status":"pass","functions":["safe_div"],"error_codes":["division_by_zero"]}"#,
    ));
}

#[test]
fn answer_verifier_retry_machine_evidence_accepts_local_code_json_with_artifact_and_validation() {
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-local-code-retry-validated",
        "ask",
        "request",
    );
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            r#"{"extra":{"action":"write_text","path":"/workspace/calc_core.py","resolved_path":"/workspace/calc_core.py"}}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "run_cmd",
            r#"{"exit_code":0,"stdout":"ok"}"#,
        ));

    assert!(answer_verifier_retry_answer_has_required_machine_evidence(
        Some(&journal),
        r#"{"changed_files":["calc_core.py"],"test_command":"python3 test_calc_core.py","test_status":"pass","functions":["safe_div"],"error_codes":["division_by_zero"]}"#,
    ));
}

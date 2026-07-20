use super::{
    answer_verifier_retry_answer_has_required_machine_evidence,
    recover_answer_verifier_gap_with_deterministic_machine_evidence, route_result,
};

use serde_json::json;

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
            r#"{"exit_code":0,"stdout":"ok","validation_result":{"status":"pass"}}"#,
        ));

    assert!(answer_verifier_retry_answer_has_required_machine_evidence(
        Some(&journal),
        r#"{"changed_files":["calc_core.py"],"test_command":"python3 test_calc_core.py","test_status":"pass","functions":["safe_div"],"error_codes":["division_by_zero"]}"#,
    ));
}

#[test]
fn verifier_gap_recovery_projects_machine_evidence_before_llm_retry() {
    let prompt = "Return only branch and remotes.";
    let route = route_result();
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-verifier-gap-machine-recovery",
        "ask",
        prompt,
    );
    journal.record_answer_verifier_summary(crate::answer_verifier::AnswerVerifierOut {
        pass: false,
        missing_evidence_fields: vec!["field_value".to_string()],
        answer_incomplete_reason: "candidate omitted observed machine fields".to_string(),
        should_retry: true,
        retry_instruction: "collect_required_evidence_fields:field_value".to_string(),
        confidence: 0.92,
    });
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "git_basic",
            json!({
                "extra": {
                    "action": "current_branch",
                    "branch": "main",
                    "field_value": {
                        "branch": "main"
                    }
                }
            })
            .to_string(),
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "git_basic",
            json!({
                "extra": {
                    "action": "remote",
                    "field_value": {
                        "remotes": ["origin"]
                    },
                    "remotes": [
                        {
                            "direction": "fetch",
                            "name": "origin",
                            "url": "git@example.com:owner/repo.git"
                        }
                    ]
                }
            })
            .to_string(),
        ));
    let mut answer_text = "branch is main and remote exists".to_string();
    let mut answer_messages = vec![answer_text.clone()];

    assert!(
        recover_answer_verifier_gap_with_deterministic_machine_evidence(
            prompt,
            &route,
            &mut journal,
            &mut answer_text,
            &mut answer_messages,
        )
    );

    assert_eq!(answer_text, r#"branch=main remotes=["origin"]"#);
    assert_eq!(answer_messages, vec![answer_text.clone()]);
    assert!(journal
        .answer_verifier_summary
        .as_ref()
        .is_some_and(|summary| summary.pass && !summary.should_retry));
    assert_eq!(journal.final_answer.as_deref(), Some(answer_text.as_str()));
}

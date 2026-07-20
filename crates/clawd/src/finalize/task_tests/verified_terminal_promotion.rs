use super::*;

fn verifier_pass_journal(task_id: &str, prompt: &str) -> crate::task_journal::TaskJournal {
    let mut journal = crate::task_journal::TaskJournal::for_task(task_id, "ask", prompt);
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: true,
        missing_evidence_fields: Vec::new(),
        answer_incomplete_reason: String::new(),
        should_retry: false,
        retry_instruction: String::new(),
        confidence: 0.96,
    });
    journal
}

#[test]
fn verifier_pass_promotes_latest_terminal_text_over_stale_machine_projection() {
    let prompt = "Inspect archive and database fixtures, then return a table.";
    let mut route = route_result();
    route.requires_content_evidence = true;
    route.response_shape = crate::OutputResponseShape::Free;
    route.locator_kind = crate::OutputLocatorKind::Path;

    let mut journal = verifier_pass_journal("task-verified-terminal-promotion", prompt);
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "archive_basic",
            r#"{"extra":{"field_value":{"members":["notes.txt","nested/config.ini"],"content_excerpt":"fixture archive notes","path":"/tmp/test_bundle.zip"}}}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "db_basic",
            r#"{"extra":{"field_value":{"tables":["orders","service_logs","users"],"schema_version":3}}}"#,
        ));
    let latest_answer = concat!(
        "| check | value |\n",
        "| --- | --- |\n",
        "| zip member 1 | notes.txt |\n",
        "| zip member 2 | nested/config.ini |\n",
        "| notes.txt | fixture archive notes |\n",
        "| tables | orders, service_logs, users |\n",
        "| schema_version | 3 |"
    );
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_3",
            "respond",
            latest_answer,
        ));

    let mut answer_text = concat!(
        "| field | value |\n",
        "| --- | --- |\n",
        "| archive.member | notes.txt |\n",
        "| db.schema_version | 3 |"
    )
    .to_string();
    let mut answer_messages = vec![answer_text.clone()];

    assert!(apply_verified_terminal_answer_after_verifier_pass(
        &route,
        &mut journal,
        &mut answer_text,
        &mut answer_messages,
        false,
    ));
    assert_eq!(answer_text, latest_answer);
    assert!(answer_text.contains("nested/config.ini"));
    assert_eq!(answer_messages, vec![latest_answer.to_string()]);
    assert_eq!(journal.final_answer.as_deref(), Some(latest_answer));
}

#[test]
fn verifier_pass_preserves_current_recorded_candidate_over_stale_terminal_step() {
    let prompt = "Return the requested machine fields with their observed values.";
    let mut route = route_result();
    route.requires_content_evidence = true;
    route.response_shape = crate::OutputResponseShape::Free;
    route.locator_kind = crate::OutputLocatorKind::Path;

    let mut journal = verifier_pass_journal("task-current-verified-candidate", prompt);
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            r##"{"extra":{"field_value":{"path":"/workspace/README.md","first_line":"# Project","line_count":1277}}}"##,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "synthesize_answer",
            "path: /workspace/READM.md\nfirst_line: # Project\nline_count: unavailable",
        ));
    let current_candidate = "path: /workspace/README.md\nfirst_line: # Project\nline_count: 1277";
    journal.record_final_answer(current_candidate);
    let mut answer_text = current_candidate.to_string();
    let mut answer_messages = vec![answer_text.clone()];

    assert!(!apply_verified_terminal_answer_after_verifier_pass(
        &route,
        &mut journal,
        &mut answer_text,
        &mut answer_messages,
        false,
    ));
    assert_eq!(answer_text, current_candidate);
    assert_eq!(answer_messages, vec![current_candidate.to_string()]);
    assert_eq!(journal.final_answer.as_deref(), Some(current_candidate));
}

#[test]
fn verifier_pass_promotes_terminal_json_over_machine_kv_projection() {
    let prompt = "Return only a JSON object with requested machine fields.";
    let mut route = route_result();
    route.requires_content_evidence = true;
    route.response_shape = crate::OutputResponseShape::Free;
    route.locator_kind = crate::OutputLocatorKind::Path;

    let mut journal = verifier_pass_journal("task-verified-terminal-json-promotion", prompt);
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "run_cmd",
            r#"{"extra":{"exit_code":0,"stdout":"OK"}}"#,
        ));
    let latest_answer = r#"{"created_files":["run/tmp/calc_core.py","run/tmp/test_calc_core.py"],"test_command":"python3 test_calc_core.py","test_status":"passed"}"#;
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "synthesize_answer",
            latest_answer,
        ));

    let mut answer_text =
        r#"created_files=["run/tmp/calc_core.py","run/tmp/test_calc_core.py"] test_command test_status=passed"#
            .to_string();
    let mut answer_messages = vec![answer_text.clone()];

    assert!(apply_verified_terminal_answer_after_verifier_pass(
        &route,
        &mut journal,
        &mut answer_text,
        &mut answer_messages,
        false,
    ));
    assert_eq!(answer_text, latest_answer);
    assert_eq!(answer_messages, vec![latest_answer.to_string()]);
    assert_eq!(journal.final_answer.as_deref(), Some(latest_answer));
}

#[test]
fn verifier_pass_does_not_promote_internal_machine_json_payload() {
    let prompt = "Return a natural summary.";
    let mut route = route_result();
    route.requires_content_evidence = true;
    route.response_shape = crate::OutputResponseShape::Free;

    let mut journal = verifier_pass_journal("task-verified-terminal-internal-json", prompt);
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "respond",
            r#"{"owner_layer":"agent_loop_control","output_format":"machine_json","status":"completed"}"#,
        ));
    let mut answer_text = "status=completed".to_string();
    let mut answer_messages = vec![answer_text.clone()];

    assert!(!apply_verified_terminal_answer_after_verifier_pass(
        &route,
        &mut journal,
        &mut answer_text,
        &mut answer_messages,
        false,
    ));
    assert_eq!(answer_text, "status=completed");
    assert_eq!(answer_messages, vec!["status=completed".to_string()]);
}

#[test]
fn verifier_recovered_terminal_answer_is_not_overwritten_by_stale_step_answer() {
    let prompt = "Read the title of ALPHA_DOC. Output only the title.";
    let mut route = route_result();
    route.requires_content_evidence = true;
    route.response_shape = crate::OutputResponseShape::OneSentence;
    route.locator_kind = crate::OutputLocatorKind::Path;

    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-verified-retry-stale-step", "ask", prompt);
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
    journal.record_final_stop_signal(
        crate::task_journal::ANSWER_VERIFIER_RECOVERED_TERMINAL_STOP_SIGNAL,
    );
    journal.record_final_answer("Service Notes");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "respond",
            "The title of ALPHA_DOC is \"Service Notes\".",
        ));

    let mut answer_text = "Service Notes".to_string();
    let mut answer_messages = vec![answer_text.clone()];

    assert!(!apply_verified_terminal_answer_after_verifier_pass(
        &route,
        &mut journal,
        &mut answer_text,
        &mut answer_messages,
        false,
    ));
    assert_eq!(answer_text, "Service Notes");
    assert_eq!(answer_messages, vec!["Service Notes".to_string()]);
    assert_eq!(journal.final_answer.as_deref(), Some("Service Notes"));
}

#[test]
fn verifier_pass_does_not_promote_terminal_text_when_machine_summary_is_required() {
    let prompt = "Return branch machine fields.";
    let mut route = route_result();
    route.response_shape = crate::OutputResponseShape::Free;
    let mut journal = verifier_pass_journal("task-verified-terminal-machine-summary", prompt);
    journal.record_turn_analysis(&crate::turn_context::TurnAnalysis {
        turn_type: Some(crate::turn_context::TurnType::TaskRequest),
        target_task_policy: Some(crate::turn_context::TargetTaskPolicy::Standalone),
        should_interrupt_active_run: false,
        state_patch: Some(serde_json::json!({
            "output_format": "machine_summary",
            "required_machine_fields": ["branch"]
        })),
        attachment_processing_required: false,
    });
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "respond",
            "branch is main",
        ));
    let mut answer_text = "branch=main".to_string();
    let mut answer_messages = vec![answer_text.clone()];

    assert!(!apply_verified_terminal_answer_after_verifier_pass(
        &route,
        &mut journal,
        &mut answer_text,
        &mut answer_messages,
        false,
    ));
    assert_eq!(answer_text, "branch=main");
    assert_eq!(answer_messages, vec!["branch=main".to_string()]);
}

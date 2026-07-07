use super::*;

fn route_result() -> crate::RouteResult {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "test".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract::default(),
    };
    route.output_contract.requires_content_evidence = true;
    route
}

#[test]
fn structured_table_recovery_handles_stale_failure_after_verifier_pass() {
    let route = route_result();
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-structured-table",
        "ask",
        "summarize observed evidence",
    );
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: true,
        missing_evidence_fields: Vec::new(),
        answer_incomplete_reason: String::new(),
        should_retry: false,
        retry_instruction: String::new(),
        confidence: 0.96,
    });
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
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_3",
            "respond",
            r#"{"archive":{"entries":["notes.txt","nested/config.ini"]},"database":{"tables":["orders","service_logs","users"]}}"#,
        ));

    let recovered = deterministic_structured_evidence_table_recovery(&route, &journal, true)
        .expect("structured table recovery");

    assert!(recovered.contains("| field | value |"));
    assert!(recovered.contains("archive.members"));
    assert!(recovered.contains("notes.txt"));
    assert!(recovered.contains("db.schema_version"));
    assert!(recovered.contains("3"));
}

#[test]
fn structured_table_recovery_ignores_passed_text_terminal_answer() {
    let route = route_result();
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-structured-table",
        "ask",
        "summarize observed evidence",
    );
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: true,
        missing_evidence_fields: Vec::new(),
        answer_incomplete_reason: String::new(),
        should_retry: false,
        retry_instruction: String::new(),
        confidence: 0.96,
    });
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "db_basic",
            r#"{"extra":{"field_value":{"tables":["orders"],"schema_version":3}}}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "respond",
            "tables: orders\nschema_version: 3",
        ));

    assert!(deterministic_structured_evidence_table_recovery(&route, &journal, true).is_none());
}

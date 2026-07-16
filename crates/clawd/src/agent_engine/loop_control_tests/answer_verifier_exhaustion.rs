use super::*;

#[test]
fn answer_verifier_exhaustion_marks_reply_failure() {
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
    journal.record_final_answer("old answer");
    let verifier = crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["output_format".to_string()],
        answer_incomplete_reason: "expected exactly five paths".to_string(),
        should_retry: true,
        retry_instruction: "select five paths".to_string(),
        confidence: 0.95,
    };
    journal.answer_verifier_summary = Some(verifier.clone());
    let mut reply = AskReply::non_llm("old answer".to_string())
        .with_messages(vec![
            "**Execution**\n1. Ran tool `fs_basic`.".to_string(),
            "old answer".to_string(),
        ])
        .with_task_journal(journal);

    mark_reply_failed_after_answer_verifier_exhausted("Find five paths", &mut reply, &verifier);

    assert!(reply.should_fail_task);
    assert_eq!(reply.messages.len(), 2);
    assert!(reply.messages[0].starts_with("**Execution**"));
    let payload: serde_json::Value =
        serde_json::from_str(&reply.text).expect("structured verifier failure payload");
    assert_eq!(
        payload
            .get("message_key")
            .and_then(serde_json::Value::as_str),
        Some("answer_verifier_required_evidence_block")
    );
    assert_eq!(
        payload
            .get("reason_code")
            .and_then(serde_json::Value::as_str),
        Some("answer_verifier_required_evidence_block")
    );
    assert_eq!(
        payload
            .pointer("/missing_evidence_fields/0")
            .and_then(serde_json::Value::as_str),
        Some("output_format")
    );
    let journal = reply.task_journal.as_ref().expect("journal");
    assert_eq!(
        journal.final_status,
        Some(crate::task_journal::TaskJournalFinalStatus::Failure)
    );
    assert_eq!(journal.final_answer.as_deref(), Some(reply.text.as_str()));
    assert_eq!(
        journal.final_failure_attribution.as_deref(),
        Some("contract_gap")
    );
}

#[test]
fn answer_verifier_exhaustion_recovers_latest_contractual_synthesis() {
    let mut route = route_result(OutputResponseShape::Free);
    route.requires_content_evidence = true;
    route.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "logs/app.log | docs/service_notes.md".to_string();
    let answer =
        "Log evidence reports warn=2 and error=1. Document evidence reports Service Notes.";
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_1".to_string(),
            skill: "log_analyze".to_string(),
            status: StepExecutionStatus::Ok,
            output_excerpt: Some(
                r#"{"keyword_counts":{"warn":2,"error":1},"path":"logs/app.log"}"#.to_string(),
            ),
            error_excerpt: None,
            started_at: 0,
            finished_at: 0,
        });
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_2".to_string(),
            skill: "doc_parse".to_string(),
            status: StepExecutionStatus::Ok,
            output_excerpt: Some(
                r##"{"extra":{"content_excerpt":"# Service Notes\nbody","path":"docs/service_notes.md"}}"##
                    .to_string(),
            ),
            error_excerpt: None,
            started_at: 0,
            finished_at: 0,
        });
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_3".to_string(),
            skill: "synthesize_answer".to_string(),
            status: StepExecutionStatus::Ok,
            output_excerpt: Some(answer.to_string()),
            error_excerpt: None,
            started_at: 0,
            finished_at: 0,
        });
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["content_excerpt".to_string()],
        answer_incomplete_reason: "previous candidate was incomplete".to_string(),
        should_retry: true,
        retry_instruction: "use observed synthesis".to_string(),
        confidence: 0.95,
    });
    let mut reply = AskReply::non_llm("previous candidate".to_string()).with_task_journal(journal);

    assert!(try_recover_latest_synthesis_answer_verifier_gap(
        Some(&answer_contract(&route)),
        &mut reply
    ));

    assert!(!reply.should_fail_task);
    assert_eq!(reply.text, answer);
    assert_eq!(reply.messages, vec![answer.to_string()]);
    let journal = reply.task_journal.as_ref().expect("journal");
    assert!(journal.answer_verifier_summary.is_none());
    assert_eq!(
        journal.final_status,
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
    assert_eq!(journal.final_answer.as_deref(), Some(answer));
}

#[test]
fn latest_synthesis_recovery_rejects_post_write_failed_validation() {
    let mut route = route_result(OutputResponseShape::Free);
    route.requires_content_evidence = true;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "/workspace/test_calc_core.py".to_string();
    let stale_answer = r#"{"created_files":["calc_core.py","test_calc_core.py"],"test_command":"python3 test_calc_core.py","test_status":"not_observed"}"#;
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-post-write-failed-validation",
        "ask",
        "prompt",
    );
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            r#"{"extra":{"action":"write_text","path":"/workspace/test_calc_core.py","resolved_path":"/workspace/test_calc_core.py"},"text":"written 120 bytes"}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::new(
            "step_2",
            "run_cmd",
            StepExecutionStatus::Error,
            None,
            Some(
                r#"__RC_SKILL_ERROR__:{"skill":"run_cmd","error_kind":"nonzero_exit","error_text":"command failed with exit code 1","extra":{"exit_code":1,"stderr":"SyntaxError"}}"#
                    .to_string(),
            ),
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_3",
            "synthesize_answer",
            stale_answer,
        ));
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["content_excerpt".to_string()],
        answer_incomplete_reason: "previous candidate missed validation success".to_string(),
        should_retry: true,
        retry_instruction: "repair and rerun validation".to_string(),
        confidence: 0.95,
    });
    let mut reply = AskReply::non_llm("previous candidate".to_string()).with_task_journal(journal);

    assert!(!try_recover_latest_synthesis_answer_verifier_gap(
        Some(&answer_contract(&route)),
        &mut reply
    ));
    assert_eq!(reply.text, "previous candidate");
    assert!(reply
        .task_journal
        .as_ref()
        .and_then(|journal| journal.answer_verifier_summary.as_ref())
        .is_some());
}

#[test]
fn answer_verifier_exhaustion_recovers_multi_source_terminal_answer_for_free_route() {
    let mut route = route_result(OutputResponseShape::Free);
    route.requires_content_evidence = false;
    route.locator_kind = OutputLocatorKind::None;
    route.locator_hint.clear();
    let terminal_answer = concat!(
        "Log analysis:\n",
        "error=1 warn=2\n",
        "Document summary:\n",
        "Service Notes contains restart guidance.\n",
        "Sorted scores:\n",
        "| name | score |\n",
        "| beta | 12 |\n",
        "| gamma | 9 |\n",
        "| alpha | 7 |"
    );
    let table_only = "| name | score |\n| beta | 12 |\n| gamma | 9 |\n| alpha | 7 |";
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-compound-terminal", "ask", "prompt");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "log_analyze",
            r#"{"keyword_counts":{"error":1,"warn":2},"path":"logs/app.log"}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "doc_parse",
            r##"{"extra":{"content_excerpt":"# Service Notes\nrestart guidance","path":"docs/service_notes.md"}}"##,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_3",
            "transform",
            r#"{"format":"markdown_table","rows":[{"name":"beta","score":12},{"name":"gamma","score":9},{"name":"alpha","score":7}]}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_4",
            "respond",
            terminal_answer,
        ));
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["content_excerpt".to_string()],
        answer_incomplete_reason: "candidate omitted required observed content".to_string(),
        should_retry: true,
        retry_instruction: "use the latest terminal answer with all observed outputs".to_string(),
        confidence: 0.92,
    });
    let mut reply = AskReply::non_llm(table_only.to_string()).with_task_journal(journal);

    assert!(try_recover_latest_synthesis_answer_verifier_gap(
        Some(&answer_contract(&route)),
        &mut reply
    ));

    assert!(!reply.should_fail_task);
    assert_eq!(reply.text, terminal_answer);
    assert_eq!(reply.messages, vec![terminal_answer.to_string()]);
    let journal = reply.task_journal.as_ref().expect("journal");
    assert!(journal.answer_verifier_summary.is_none());
    assert_eq!(
        journal.final_status,
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
    assert_eq!(journal.final_answer.as_deref(), Some(terminal_answer));
}

#[test]
fn structured_count_recovery_returns_machine_fields_without_language_template() {
    let mut route = route_result(OutputResponseShape::Scalar);
    route.semantic_kind = OutputSemanticKind::ScalarCount;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-count-recovery", "ask", "prompt");
    journal.step_results.push(
        crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            r#"{"action":"count_inventory","path":"docs","recursive":false,"counts":{"total":3,"files":2,"dirs":1,"hidden":0}}"#,
        ),
    );
    let mut reply = AskReply::non_llm("old count answer".to_string()).with_task_journal(journal);

    assert!(try_recover_structured_count_answer_verifier_gap(
        Some(&answer_contract(&route)),
        "数一下 docs 下面有多少项",
        &mut reply,
    ));

    assert!(reply
        .text
        .contains("message_key=clawd.msg.structured_count.summary"));
    assert!(reply.text.contains("reason_code=structured_count_observed"));
    assert!(reply.text.contains("path=docs"));
    assert!(reply.text.contains("total=3"));
    assert!(reply.text.contains("files=2"));
    assert!(reply.text.contains("dirs=1"));
    assert!(reply.text.contains("hidden=0"));
    assert!(!reply.text.contains("共有"), "reply: {}", reply.text);
    assert!(!reply.text.contains("has "), "reply: {}", reply.text);
}

#[test]
fn structured_search_recovery_returns_machine_candidates_without_language_template() {
    let mut route = route_result(OutputResponseShape::Free);
    route.semantic_kind = OutputSemanticKind::FileNames;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-search-recovery", "ask", "prompt");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            r#"{"action":"find_name","count":2,"results":["README.md","README.zh-CN.md"]}"#,
        ));
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["candidates".to_string()],
        answer_incomplete_reason: "candidate omitted observed names".to_string(),
        should_retry: true,
        retry_instruction: "render all candidates".to_string(),
        confidence: 0.95,
    });
    let mut reply = AskReply::non_llm("README".to_string()).with_task_journal(journal);

    assert!(try_recover_structured_search_answer_verifier_gap(
        Some(&answer_contract(&route)),
        "找 README 文件",
        &mut reply,
    ));

    assert!(reply
        .text
        .contains("message_key=clawd.msg.structured_search.candidates"));
    assert!(reply
        .text
        .contains("reason_code=structured_search_candidates"));
    assert!(reply.text.contains("action=find_name"));
    assert!(reply.text.contains("count=2"));
    assert!(reply.text.contains("result_count=2"));
    assert!(reply.text.contains("candidate.1=README.md"));
    assert!(reply.text.contains("candidate.2=README.zh-CN.md"));
    assert!(!reply.text.contains("找到"), "reply: {}", reply.text);
    assert!(!reply.text.contains("Found"), "reply: {}", reply.text);
}

#[test]
fn answer_verifier_exhaustion_recovers_filesystem_mutation_success_payload() {
    let mut route = route_result(OutputResponseShape::OneSentence);
    route.semantic_kind = OutputSemanticKind::FilesystemMutationResult;
    route.requires_content_evidence = false;
    route.locator_hint = "README.md".to_string();
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-filesystem-mutation-success",
        "ask",
        "prompt",
    );
    journal.step_results.push(crate::task_journal::TaskJournalStepTrace::ok(
        "step_1",
        "kb",
        r#"{"request_id":"req-1","status":"ok","text":"already_indexed","error_text":null,"extra":{"action":"ingest","namespace":"demo_docs_nl","path":"README.md","effective_status":"ok","result_kind":"already_indexed","effective_success":true,"idempotent_success":true,"stats":{"total_chunks":59}}}"#,
    ));
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["output_format".to_string()],
        answer_incomplete_reason: "candidate did not render the machine success payload"
            .to_string(),
        should_retry: true,
        retry_instruction: "render observed success fields".to_string(),
        confidence: 0.95,
    });
    let mut reply = AskReply::non_llm(
        r#"{"contract_marker":"filesystem_mutation_result","status":"ok","effective_status":"ok","effective_success":true,"idempotent_success":true,"result_kinds":["already_indexed"],"paths":["README.md"],"namespaces":["demo_docs_nl"],"steps":[{"status":"ok","action":"ingest","path":"README.md","namespace":"demo_docs_nl","result_kind":"already_indexed","stats":{"total_chunks":59}}]}"#
            .to_string(),
    )
    .with_task_journal(journal);

    assert!(try_recover_filesystem_mutation_success_answer_verifier_gap(
        Some(&answer_contract(&route)),
        &mut reply
    ));

    assert!(!reply.should_fail_task);
    assert_eq!(
        reply.text,
        "status=ok effective_status=ok result_kind=already_indexed action=ingest path=README.md namespace=demo_docs_nl total_chunks=59"
    );
    assert_eq!(reply.messages, vec![reply.text.clone()]);
    let journal = reply.task_journal.as_ref().expect("journal");
    assert!(journal.answer_verifier_summary.is_none());
    assert_eq!(
        journal.final_status,
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
    assert_eq!(journal.final_answer.as_deref(), Some(reply.text.as_str()));
}

#[test]
fn answer_verifier_exhaustion_recovers_latest_terminal_respond_after_retry() {
    let mut route = route_result(OutputResponseShape::Free);
    route.requires_content_evidence = true;
    route.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "logs/app.log | docs/service_notes.md".to_string();
    let corrected_answer =
        "Log evidence reports warn=2 and error=1. Document evidence reports Service Notes.";
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            r#"{"action":"read_range","path":"logs/app.log","excerpt":"1|WARN latency\n2|ERROR timeout"}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "fs_basic",
            r##"{"action":"read_range","path":"docs/service_notes.md","excerpt":"1|# Service Notes\n2|body"}"##,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_3",
            "synthesize_answer",
            "Old candidate included an unsupported section.",
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_4",
            "respond",
            corrected_answer,
        ));
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["unsupported_claims".to_string()],
        answer_incomplete_reason: "previous candidate had unsupported claims".to_string(),
        should_retry: true,
        retry_instruction: "use the corrected terminal answer".to_string(),
        confidence: 0.9,
    });
    let mut reply =
        AskReply::non_llm("answer verifier fallback".to_string()).with_task_journal(journal);

    assert!(try_recover_latest_synthesis_answer_verifier_gap(
        Some(&answer_contract(&route)),
        &mut reply
    ));

    assert!(!reply.should_fail_task);
    assert_eq!(reply.text, corrected_answer);
    assert_eq!(reply.messages, vec![corrected_answer.to_string()]);
    let journal = reply.task_journal.as_ref().expect("journal");
    assert!(journal.answer_verifier_summary.is_none());
    assert_eq!(
        journal.final_status,
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
}

#[test]
fn answer_verifier_exhaustion_recovers_structured_archive_db_table() {
    let mut route = route_result(OutputResponseShape::Free);
    route.requires_content_evidence = true;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "tmp/test_bundle.zip | data/test_contract.sqlite".to_string();

    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-archive-db", "ask", "prompt");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "archive_basic",
            r#"{"extra":{"action":"list","archive":"tmp/test_bundle.zip","field_value":{"members":["notes.txt","nested/config.ini"],"member_count":2,"count":2}},"text":"{}"}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "archive_basic",
            r#"{"extra":{"action":"read","field_value":{"path":"notes.txt","content_excerpt":"fixture archive notes"},"content_excerpt":"fixture archive notes"},"text":"{}"}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_3",
            "db_basic",
            r#"{"extra":{"action":"list_tables","db_path":"data/test_contract.sqlite","field_value":{"table_count":3,"tables":["orders","service_logs","users"]},"tables":["orders","service_logs","users"]},"text":"{}"}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_4",
            "db_basic",
            r#"{"extra":{"action":"schema_version","db_path":"data/test_contract.sqlite","field_value":{"schema_version":3},"schema_version":3},"text":"{}"}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_5",
            "respond",
            r#"{"archive":{"entries":["notes.txt","nested/config.ini"]},"database":{"tables":["orders","service_logs","users"]}}"#,
        ));
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["output_format".to_string(), "field_value".to_string()],
        answer_incomplete_reason: "candidate missed required structured projection".to_string(),
        should_retry: true,
        retry_instruction: "render observed field_value facts as a table".to_string(),
        confidence: 0.92,
    });
    let mut reply = AskReply::non_llm(
        r#"{"archive":{"entries":["notes.txt","nested/config.ini"]},"database":{"tables":["orders","service_logs","users"]}}"#
            .to_string(),
    )
    .with_task_journal(journal);

    assert!(try_recover_structured_evidence_table_answer_verifier_gap(
        Some(&answer_contract(&route)),
        &mut reply
    ));

    assert!(!reply.should_fail_task);
    assert!(reply.text.starts_with("| field | value |"));
    assert!(reply.text.contains("archive.members"));
    assert!(reply.text.contains("notes.txt, nested/config.ini"));
    assert!(reply.text.contains("db.tables"));
    assert!(reply.text.contains("orders, service_logs, users"));
    assert!(reply.text.contains("db.schema_version"));
    assert!(reply.text.contains("| 3 |"));
    let journal = reply.task_journal.as_ref().expect("journal");
    assert!(journal.answer_verifier_summary.is_none());
    assert_eq!(
        journal.final_status,
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
}

#[test]
fn answer_verifier_exhaustion_does_not_recover_unstructured_terminal_for_field_value_gap() {
    let mut route = route_result(OutputResponseShape::Free);
    route.requires_content_evidence = true;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "tmp/test_bundle.zip | data/test_contract.sqlite".to_string();

    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-archive-db", "ask", "prompt");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "archive_basic",
            r#"{"extra":{"field_value":{"members":["notes.txt","nested/config.ini"],"member_count":2}}}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "archive_basic",
            r#"{"extra":{"field_value":{"path":"notes.txt","content_excerpt":"fixture archive notes"}}}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_3",
            "db_basic",
            r#"{"extra":{"field_value":{"table_count":3,"tables":["orders","service_logs","users"]}}}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_4",
            "db_basic",
            r#"{"extra":{"field_value":{"schema_version":3},"schema_version":3}}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_5",
            "synthesize_answer",
            r#"{"archive":{"entries":["notes.txt","nested/config.ini"]},"database":{"tables":["orders","service_logs","users"]}}"#,
        ));
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["output_format".to_string(), "field_value".to_string()],
        answer_incomplete_reason: "candidate missed required structured projection".to_string(),
        should_retry: true,
        retry_instruction: "render observed field_value facts as a table".to_string(),
        confidence: 0.92,
    });
    let mut reply = AskReply::non_llm("previous candidate".to_string()).with_task_journal(journal);

    assert!(!try_recover_latest_synthesis_answer_verifier_gap(
        Some(&answer_contract(&route)),
        &mut reply
    ));
    assert_eq!(reply.text, "previous candidate");
}

#[test]
fn answer_verifier_exhaustion_does_not_recover_same_rejected_terminal_respond() {
    let mut route = route_result(OutputResponseShape::Free);
    route.requires_content_evidence = true;
    route.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "docs/service_notes.md".to_string();
    let rejected_answer = "Candidate includes an unsupported section.";
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            r##"{"action":"read_range","path":"docs/service_notes.md","excerpt":"1|# Service Notes\n2|body"}"##,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "respond",
            rejected_answer,
        ));
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["unsupported_claims".to_string()],
        answer_incomplete_reason: "current candidate has unsupported claims".to_string(),
        should_retry: true,
        retry_instruction: "remove unsupported claims".to_string(),
        confidence: 0.9,
    });
    let mut reply = AskReply::non_llm(rejected_answer.to_string()).with_task_journal(journal);

    assert!(!try_recover_latest_synthesis_answer_verifier_gap(
        Some(&answer_contract(&route)),
        &mut reply
    ));
}

#[test]
fn unsupported_claims_gap_requests_observed_content_rewrite() {
    let verifier = crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["unsupported_claims".to_string()],
        answer_incomplete_reason: "candidate added unobserved facts".to_string(),
        should_retry: true,
        retry_instruction: "rewrite from observed evidence".to_string(),
        confidence: 0.88,
    };

    assert!(answer_verifier_gap_requests_observed_content_rewrite(
        &verifier
    ));
}

#[test]
fn observed_content_rewrite_gate_uses_only_successful_nonterminal_evidence() {
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-content-rewrite", "ask", "prompt");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::new(
            "step_1",
            "system_basic",
            StepExecutionStatus::Error,
            None,
            Some(
                r#"{"error_text":"content_excerpt is not machine evidence on failed steps"}"#
                    .to_string(),
            ),
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "respond",
            "terminal answer with content_excerpt wording is not observation evidence",
        ));

    assert!(!answer_verifier_gap_has_observed_content_evidence(&journal));

    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_3",
            "fs_basic",
            r#"{"extra":{"action":"read_range","path":"README.md","excerpt":"1|# RustClaw\n2|local runtime"}}"#,
        ));

    assert!(answer_verifier_gap_has_observed_content_evidence(&journal));
}

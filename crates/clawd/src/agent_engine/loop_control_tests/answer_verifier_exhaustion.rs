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
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "logs/app.log | docs/service_notes.md".to_string();
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
        Some(&route),
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
fn answer_verifier_exhaustion_recovers_filesystem_mutation_success_payload() {
    let mut route = route_result(OutputResponseShape::OneSentence);
    route.output_contract.semantic_kind = OutputSemanticKind::FilesystemMutationResult;
    route.output_contract.requires_content_evidence = false;
    route.output_contract.locator_hint = "README.md".to_string();
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
        r#"{"semantic_kind":"filesystem_mutation_result","status":"ok","effective_status":"ok","effective_success":true,"idempotent_success":true,"result_kinds":["already_indexed"],"paths":["README.md"],"namespaces":["demo_docs_nl"],"steps":[{"status":"ok","action":"ingest","path":"README.md","namespace":"demo_docs_nl","result_kind":"already_indexed","stats":{"total_chunks":59}}]}"#
            .to_string(),
    )
    .with_task_journal(journal);

    assert!(try_recover_filesystem_mutation_success_answer_verifier_gap(
        Some(&route),
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
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "logs/app.log | docs/service_notes.md".to_string();
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
        Some(&route),
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
fn answer_verifier_exhaustion_does_not_recover_same_rejected_terminal_respond() {
    let mut route = route_result(OutputResponseShape::Free);
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "docs/service_notes.md".to_string();
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
        Some(&route),
        &mut reply
    ));
}

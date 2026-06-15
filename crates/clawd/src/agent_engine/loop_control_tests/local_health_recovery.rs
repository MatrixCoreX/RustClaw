use super::*;

#[test]
fn local_health_verifier_gap_recovers_with_machine_fields() {
    let mut route = route_result(OutputResponseShape::Free);
    route.output_contract.semantic_kind = OutputSemanticKind::CommandOutputSummary;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.output_contract.requires_content_evidence = true;

    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-health", "ask", "health summary");
    journal.step_results.push(crate::task_journal::TaskJournalStepTrace {
        step_id: "step_1".to_string(),
        skill: "run_cmd".to_string(),
        status: StepExecutionStatus::Ok,
        output_excerpt: Some(
            "Filesystem      Size  Used Avail Use% Mounted on\n/dev/nvme0n1p6  146G  132G  6.4G  96% /\n"
                .to_string(),
        ),
        error_excerpt: None,
        started_at: 0,
        finished_at: 0,
    });
    journal.step_results.push(crate::task_journal::TaskJournalStepTrace {
        step_id: "step_2".to_string(),
        skill: "run_cmd".to_string(),
        status: StepExecutionStatus::Ok,
        output_excerpt: Some(
            "Mem:            14Gi       5.6Gi       3.7Gi       1.1Gi       6.9Gi       9.2Gi\nSwap:          4.0Gi       3.8Gi       157Mi\n"
                .to_string(),
        ),
        error_excerpt: None,
        started_at: 0,
        finished_at: 0,
    });
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["command_output".to_string()],
        answer_incomplete_reason: "candidate omitted observed health fields".to_string(),
        should_retry: true,
        retry_instruction: "render observed health fields".to_string(),
        confidence: 0.95,
    });
    let mut reply = AskReply::non_llm("incomplete".to_string()).with_task_journal(journal);

    assert!(try_recover_local_health_answer_verifier_gap(
        Some(&route),
        &mut reply
    ));

    assert!(!reply.should_fail_task);
    assert!(reply.text.contains("disk_root_use_percent=96%"));
    assert!(reply.text.contains("disk_root_available=6.4G"));
    assert!(reply.text.contains("memory_total=14Gi"));
    assert!(reply.text.contains("memory_used=5.6Gi"));
    assert!(reply.text.contains("memory_available=9.2Gi"));
    assert!(reply.text.contains("swap_used=3.8Gi"));
    assert_eq!(
        reply
            .task_journal
            .as_ref()
            .and_then(|journal| journal.final_status),
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
}

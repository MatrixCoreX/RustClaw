use super::*;

#[test]
fn recent_artifacts_verifier_gap_recovers_from_inventory_metadata() {
    let mut route = route_result(OutputResponseShape::Free);
    route.output_contract.semantic_kind = OutputSemanticKind::RecentArtifactsJudgment;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.requires_content_evidence = true;

    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-recent-artifacts", "ask", "judge tmp");
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Failure);
    journal.step_results.push(crate::task_journal::TaskJournalStepTrace {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output_excerpt: Some(
            r#"{"extra":{"action":"inventory_dir","counts":{"dirs":2,"files":1,"total":3},"entries":[{"kind":"file","modified_ts":1781480234,"name":"test_bundle.zip","path":"scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip","size_bytes":272},{"kind":"dir","modified_ts":1781462084,"name":"clarify_unpack_case","path":"scripts/nl_tests/fixtures/device_local/tmp/clarify_unpack_case","size_bytes":0},{"kind":"dir","modified_ts":1781137621,"name":"manual_dynamic_guard_unpack","path":"scripts/nl_tests/fixtures/device_local/tmp/manual_dynamic_guard_unpack","size_bytes":0}],"path":"/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/tmp","sort_by":"mtime_desc"},"text":"metadata"}"#.to_string(),
        ),
        error_excerpt: None,
        started_at: 0,
        finished_at: 0,
    });
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["output_format".to_string()],
        answer_incomplete_reason: "candidate leaked raw step output".to_string(),
        should_retry: true,
        retry_instruction: "render observed recent entries".to_string(),
        confidence: 0.95,
    });
    let mut reply = AskReply::non_llm("incomplete".to_string()).with_task_journal(journal);

    assert!(try_recover_recent_artifacts_answer_verifier_gap(
        Some(&route),
        &mut reply
    ));

    assert!(!reply.should_fail_task);
    assert!(reply.text.contains("recent_entries.count=3"));
    assert!(reply
        .text
        .contains("recent_entries[0].name=test_bundle.zip"));
    assert!(reply.text.contains("recent_entries[0].extension=zip"));
    assert!(reply.text.contains("recent_entries[1].kind=dir"));
    assert!(reply
        .text
        .contains("classification.temporary_bundle_artifact=true"));
    assert!(reply.text.contains("classification.formal_config=false"));
    assert_eq!(
        reply
            .task_journal
            .as_ref()
            .and_then(|journal| journal.final_status),
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
    assert!(reply
        .task_journal
        .as_ref()
        .and_then(|journal| journal.answer_verifier_summary.as_ref())
        .is_none());
}

#[test]
fn recent_artifacts_verifier_gap_ignores_visible_text_inventory_metadata() {
    let mut route = route_result(OutputResponseShape::Free);
    route.output_contract.semantic_kind = OutputSemanticKind::RecentArtifactsJudgment;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.requires_content_evidence = true;

    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-recent-artifacts-text-only",
        "ask",
        "judge tmp",
    );
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Failure);
    journal.step_results.push(crate::task_journal::TaskJournalStepTrace {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output_excerpt: Some(
            serde_json::json!({
                "status": "ok",
                "text": serde_json::json!({
                    "action": "inventory_dir",
                    "entries": [
                        {"kind": "file", "modified_ts": 9, "name": "clawd.run.log", "path": "logs/clawd.run.log", "size_bytes": 2048}
                    ],
                    "path": "/repo/logs",
                    "sort_by": "mtime_desc"
                })
                .to_string()
            })
            .to_string(),
        ),
        error_excerpt: None,
        started_at: 0,
        finished_at: 0,
    });
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["output_format".to_string()],
        answer_incomplete_reason: "candidate leaked raw step output".to_string(),
        should_retry: true,
        retry_instruction: "render observed recent entries".to_string(),
        confidence: 0.95,
    });
    let mut reply = AskReply::non_llm("incomplete".to_string()).with_task_journal(journal);

    assert!(!try_recover_recent_artifacts_answer_verifier_gap(
        Some(&route),
        &mut reply
    ));
    assert_eq!(reply.text, "incomplete");
}

#[test]
fn recent_artifacts_verifier_gap_recovery_respects_selector_limit_and_target_kind() {
    let mut route = route_result(OutputResponseShape::Free);
    route.output_contract.semantic_kind = OutputSemanticKind::RecentArtifactsJudgment;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.requires_content_evidence = true;
    route
        .output_contract
        .self_extension
        .list_selector
        .target_kind = crate::OutputScalarCountTargetKind::File;
    route
        .output_contract
        .self_extension
        .list_selector
        .target_kind_specified = true;
    route.output_contract.self_extension.list_selector.limit = Some(1);

    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-recent-artifacts-limit",
        "ask",
        "judge logs",
    );
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Failure);
    journal.step_results.push(crate::task_journal::TaskJournalStepTrace {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output_excerpt: Some(
            r#"{"extra":{"action":"inventory_dir","entries":[{"kind":"file","modified_ts":9,"name":"clawd.run.log","path":"logs/clawd.run.log","size_bytes":2048},{"kind":"dir","modified_ts":8,"name":"agent_rollout_metrics","path":"logs/agent_rollout_metrics","size_bytes":0},{"kind":"file","modified_ts":7,"name":"model_io.log","path":"logs/model_io.log","size_bytes":4096}],"path":"/repo/logs","sort_by":"mtime_desc"},"text":"metadata"}"#.to_string(),
        ),
        error_excerpt: None,
        started_at: 0,
        finished_at: 0,
    });
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["output_format".to_string()],
        answer_incomplete_reason: "candidate leaked raw step output".to_string(),
        should_retry: true,
        retry_instruction: "render observed recent entries".to_string(),
        confidence: 0.95,
    });
    let mut reply = AskReply::non_llm("incomplete".to_string()).with_task_journal(journal);

    assert!(try_recover_recent_artifacts_answer_verifier_gap(
        Some(&route),
        &mut reply
    ));

    assert!(reply.text.contains("recent_entries.count=1"));
    assert!(reply.text.contains("recent_entries[0].name=clawd.run.log"));
    assert!(!reply.text.contains("agent_rollout_metrics"));
    assert!(!reply.text.contains("recent_entries[1]"));
}

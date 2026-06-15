use super::{
    answer_verifier_retry_summary, ok_step, route_result,
    suppress_answer_verifier_retry_if_structurally_satisfied,
    suppress_answer_verifier_retry_if_user_locator_disambiguation,
};
use crate::{
    executor::StepExecutionStatus, AskReply, OutputDeliveryIntent, OutputResponseShape,
    OutputSemanticKind,
};
use serde_json::json;

#[test]
fn answer_verifier_retry_summary_requires_retryable_high_confidence_gap() {
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["path".to_string()],
        answer_incomplete_reason: "missing fallback path".to_string(),
        should_retry: true,
        retry_instruction: "search fallback path".to_string(),
        confidence: 0.8,
    });
    let reply = AskReply::non_llm("wrong path".to_string()).with_task_journal(journal);

    let summary = answer_verifier_retry_summary(&reply, None).expect("retry gap");
    assert_eq!(summary.missing_evidence_fields, vec!["path"]);
}

#[test]
fn answer_verifier_retry_summary_uses_high_confidence_gap_even_without_flag() {
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: Vec::new(),
        answer_incomplete_reason: "candidate contradicts observed evidence".to_string(),
        should_retry: false,
        retry_instruction: String::new(),
        confidence: 0.95,
    });
    let reply = AskReply::non_llm("wrong answer".to_string()).with_task_journal(journal);

    assert!(answer_verifier_retry_summary(&reply, None).is_some());
}

#[test]
fn answer_verifier_retry_summary_respects_explicit_retry_flag() {
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: Vec::new(),
        answer_incomplete_reason: "answer omitted requested synthesis".to_string(),
        should_retry: true,
        retry_instruction: "include the requested synthesis".to_string(),
        confidence: 0.2,
    });
    let reply = AskReply::non_llm("single candidate".to_string()).with_task_journal(journal);

    assert!(answer_verifier_retry_summary(&reply, None).is_some());
}

#[test]
fn answer_verifier_retry_summary_skips_file_delivery_candidate_disambiguation() {
    let mut route = route_result(OutputResponseShape::FileToken);
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = OutputDeliveryIntent::FileSingle;
    route.output_contract.semantic_kind = OutputSemanticKind::GeneratedFileDelivery;
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal.push_step_result(&ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"find_name","count":3,"results":["docs/a.md","docs/b.md","docs/c.md"],"root":""},"text":"{}"}"#,
    ));
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["path".to_string()],
        answer_incomplete_reason: "single file path not selected".to_string(),
        should_retry: true,
        retry_instruction: "wait for user locator selection".to_string(),
        confidence: 0.88,
    });
    let mut reply = AskReply::non_llm("multiple candidates".to_string()).with_task_journal(journal);

    assert!(answer_verifier_retry_summary(&reply, Some(&route)).is_none());
    assert!(
        suppress_answer_verifier_retry_if_user_locator_disambiguation(&mut reply, Some(&route))
    );
    assert!(reply
        .task_journal
        .as_ref()
        .and_then(|journal| journal.answer_verifier_summary.as_ref())
        .is_none());
}

#[test]
fn answer_verifier_retry_summary_skips_clarify_final_status() {
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Clarify);
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["path".to_string()],
        answer_incomplete_reason: "missing fallback path".to_string(),
        should_retry: true,
        retry_instruction: "search fallback path".to_string(),
        confidence: 0.8,
    });
    let reply = AskReply::non_llm("please provide the path".to_string()).with_task_journal(journal);

    assert!(answer_verifier_retry_summary(&reply, None).is_none());
}

#[test]
fn quantity_comparison_structural_answer_suppresses_false_verifier_retry() {
    let mut route = route_result(OutputResponseShape::OneSentence);
    route.output_contract.semantic_kind = OutputSemanticKind::QuantityComparison;
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal.step_results.push(crate::task_journal::TaskJournalStepTrace {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output_excerpt: Some(
            r#"{"action":"path_batch_facts","facts":[{"exists":true,"fact":{"path":"Cargo.lock","size_bytes":121647}},{"exists":true,"fact":{"path":"Cargo.toml","size_bytes":2606}}]}"#
                .to_string(),
        ),
        error_excerpt: None,
        started_at: 0,
        finished_at: 0,
    });
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: Vec::new(),
        answer_incomplete_reason: "answer only reports the file sizes without ratio".to_string(),
        should_retry: true,
        retry_instruction: "calculate the ratio".to_string(),
        confidence: 0.95,
    });
    let mut reply = AskReply::non_llm(
        "Cargo.lock 大小为 121,647 字节，Cargo.toml 大小为 2,606 字节。Cargo.lock 大约是 Cargo.toml 的 46.7 倍。"
            .to_string(),
    )
    .with_messages(vec![
        "**执行过程**\n1. 调用工具 `fs_basic`。".to_string(),
        "Cargo.lock 大小为 121,647 字节，Cargo.toml 大小为 2,606 字节。Cargo.lock 大约是 Cargo.toml 的 46.7 倍。"
            .to_string(),
    ])
    .with_task_journal(journal);

    assert!(suppress_answer_verifier_retry_if_structurally_satisfied(
        &mut reply,
        Some(&route)
    ));
    assert!(answer_verifier_retry_summary(&reply, Some(&route)).is_none());
    assert!(reply
        .task_journal
        .as_ref()
        .and_then(|journal| journal.answer_verifier_summary.as_ref())
        .is_none());
}

#[test]
fn quantity_comparison_suppression_reads_total_size_bytes() {
    let mut route = route_result(OutputResponseShape::OneSentence);
    route.output_contract.semantic_kind = OutputSemanticKind::QuantityComparison;
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal.step_results.push(crate::task_journal::TaskJournalStepTrace {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output_excerpt: Some(
            r#"{"action":"path_batch_facts","facts":[{"exists":true,"fact":{"path":"target","size_bytes":4096}}]}"#
                .to_string(),
        ),
        error_excerpt: None,
        started_at: 0,
        finished_at: 0,
    });
    journal.step_results.push(crate::task_journal::TaskJournalStepTrace {
        step_id: "step_2".to_string(),
        skill: "fs_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output_excerpt: Some(
            r#"{"action":"count_inventory","counts":{"total":129116,"total_size_bytes":57264444014}}"#
                .to_string(),
        ),
        error_excerpt: None,
        started_at: 0,
        finished_at: 0,
    });
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["size_bytes".to_string()],
        answer_incomplete_reason: "size evidence not visible".to_string(),
        should_retry: true,
        retry_instruction: "collect size evidence".to_string(),
        confidence: 0.95,
    });
    let mut reply =
        AskReply::non_llm("target 目录大小约 53.3 GB，包含 129116 个项目。".to_string())
            .with_task_journal(journal);

    assert!(suppress_answer_verifier_retry_if_structurally_satisfied(
        &mut reply,
        Some(&route)
    ));
    assert!(answer_verifier_retry_summary(&reply, Some(&route)).is_none());
}

#[test]
fn permission_denied_content_access_suppresses_missing_evidence_retry() {
    let mut route = route_result(OutputResponseShape::Strict);
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route.output_contract.locator_hint = "/etc/shadow".to_string();
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-permission-denied", "ask", "prompt");
    journal.step_results.push(crate::task_journal::TaskJournalStepTrace {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: StepExecutionStatus::Error,
        output_excerpt: None,
        error_excerpt: Some(format!(
            "__RC_SKILL_ERROR__:{}",
            serde_json::json!({
                "skill": "system_basic",
                "error_kind": "permission_denied",
                "error_text": "read_file failed for /etc/shadow: Permission denied (os error 13)",
                "extra": {
                    "operation": "read_file",
                    "path": "/etc/shadow"
                }
            })
        )),
        started_at: 0,
        finished_at: 0,
    });
    journal.record_finalizer_summary(crate::task_journal::TaskJournalFinalizerSummary {
        stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
        disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
        contract_ok: true,
        completion_ok: Some(true),
        grounded_ok: Some(true),
        format_ok: Some(true),
        needs_clarify: Some(false),
        used_evidence_ids_count: 1,
        ..Default::default()
    });
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["any_of(command_output|content_excerpt|field_value)".to_string()],
        answer_incomplete_reason:
            "missing required execution evidence: any_of(command_output|content_excerpt|field_value)"
                .to_string(),
        should_retry: true,
        retry_instruction: "collect content evidence".to_string(),
        confidence: 0.95,
    });
    let mut reply =
        AskReply::non_llm("已尝试访问 `/etc/shadow`，但执行失败：Permission denied。".to_string())
            .with_task_journal(journal);

    assert!(suppress_answer_verifier_retry_if_structurally_satisfied(
        &mut reply,
        Some(&route)
    ));
    assert!(answer_verifier_retry_summary(&reply, Some(&route)).is_none());
    assert!(!reply.should_fail_task);
}

#[test]
fn file_token_delivery_suppresses_list_count_verifier_retry_when_grounded() {
    let root = std::env::temp_dir().join(format!(
        "rustclaw-loop-control-file-token-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).expect("create temp root");
    let file = root.join("report.txt");
    std::fs::write(&file, "report").expect("write temp file");

    let mut route = route_result(OutputResponseShape::FileToken);
    route.wants_file_delivery = true;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = OutputDeliveryIntent::FileSingle;
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_1".to_string(),
            skill: "fs_basic".to_string(),
            status: StepExecutionStatus::Ok,
            output_excerpt: Some(
                json!({
                    "action": "inventory_dir",
                    "resolved_path": root.display().to_string(),
                    "names": ["report.txt", "other.txt"],
                    "entries": [
                        {
                            "kind": "file",
                            "name": "report.txt",
                            "path": file.display().to_string()
                        }
                    ]
                })
                .to_string(),
            ),
            error_excerpt: None,
            started_at: 0,
            finished_at: 0,
        });
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: Vec::new(),
        answer_incomplete_reason:
            "answer provides only 1 file path when evidence shows the directory contains many files"
                .to_string(),
        should_retry: true,
        retry_instruction: "list all files".to_string(),
        confidence: 0.95,
    });
    let mut reply = AskReply::non_llm(format!("FILE:{}", file.display()))
        .with_messages(vec![
            "**执行过程**\n1. 调用工具 `fs_basic`。".to_string(),
            format!("FILE:{}", file.display()),
        ])
        .with_task_journal(journal);

    assert!(suppress_answer_verifier_retry_if_structurally_satisfied(
        &mut reply,
        Some(&route)
    ));
    assert!(answer_verifier_retry_summary(&reply, Some(&route)).is_none());

    let _ = std::fs::remove_file(&file);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn file_token_delivery_does_not_suppress_when_token_is_not_grounded() {
    let root = std::env::temp_dir().join(format!(
        "rustclaw-loop-control-file-token-ungrounded-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).expect("create temp root");
    let observed = root.join("observed.txt");
    let ungrounded = root.join("ungrounded.txt");
    std::fs::write(&observed, "observed").expect("write observed file");
    std::fs::write(&ungrounded, "ungrounded").expect("write ungrounded file");

    let mut route = route_result(OutputResponseShape::FileToken);
    route.wants_file_delivery = true;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = OutputDeliveryIntent::FileSingle;
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_1".to_string(),
            skill: "fs_basic".to_string(),
            status: StepExecutionStatus::Ok,
            output_excerpt: Some(
                json!({
                    "action": "inventory_dir",
                    "resolved_path": root.display().to_string(),
                    "entries": [
                        {
                            "kind": "file",
                            "name": "observed.txt",
                            "path": observed.display().to_string()
                        }
                    ]
                })
                .to_string(),
            ),
            error_excerpt: None,
            started_at: 0,
            finished_at: 0,
        });
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: Vec::new(),
        answer_incomplete_reason: "candidate file is not supported by evidence".to_string(),
        should_retry: true,
        retry_instruction: "select a grounded file".to_string(),
        confidence: 0.95,
    });
    let mut reply =
        AskReply::non_llm(format!("FILE:{}", ungrounded.display())).with_task_journal(journal);

    assert!(!suppress_answer_verifier_retry_if_structurally_satisfied(
        &mut reply,
        Some(&route)
    ));
    assert!(answer_verifier_retry_summary(&reply, Some(&route)).is_some());

    let _ = std::fs::remove_file(&observed);
    let _ = std::fs::remove_file(&ungrounded);
    let _ = std::fs::remove_dir_all(&root);
}

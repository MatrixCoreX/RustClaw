use super::{
    answer_verifier_output_format_machine_payload_gap, answer_verifier_retry_summary,
    commit_answer_verifier_retry_answer, ok_step,
    prefer_terminal_model_answer_for_verifier_candidate, retry_verifier_accepts_rewritten_answer,
    route_result, suppress_answer_verifier_retry_if_confirmed_missing_file_delivery,
    suppress_answer_verifier_retry_if_structurally_satisfied,
    suppress_answer_verifier_retry_if_user_locator_disambiguation,
};
use crate::{
    executor::StepExecutionStatus, AskReply, OutputDeliveryIntent, OutputLocatorKind,
    OutputResponseShape, OutputSemanticKind,
};
use serde_json::json;

#[test]
fn output_format_machine_payload_gap_detects_structured_or_field_projection_reply() {
    let verifier = crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["output_format".to_string()],
        answer_incomplete_reason: "visible answer shape mismatch".to_string(),
        should_retry: true,
        retry_instruction: "render observed machine evidence".to_string(),
        confidence: 0.9,
    };

    assert!(answer_verifier_output_format_machine_payload_gap(
        &verifier,
        r#"{"message_key":"clawd.msg.config_edit.guard","candidates":["tools.allow_sudo=true"]}"#
    ));
    assert!(answer_verifier_output_format_machine_payload_gap(
        &verifier,
        r#"{"contract_marker":"filesystem_mutation_result","status":"ok","steps":[{"action":"ingest","path":"README.md"}]}"#
    ));
    assert!(answer_verifier_output_format_machine_payload_gap(
        &verifier,
        "target=telegramd service_name=telegramd post_state=telegramd=running verified=true"
    ));
    assert!(!answer_verifier_output_format_machine_payload_gap(
        &verifier,
        "configs/config.toml has one observed risk."
    ));
}

#[test]
fn retry_verifier_pass_accepts_rewritten_answer() {
    let accepted = crate::answer_verifier::AnswerVerifierOut {
        pass: true,
        missing_evidence_fields: Vec::new(),
        answer_incomplete_reason: String::new(),
        should_retry: false,
        retry_instruction: String::new(),
        confidence: 0.95,
    };
    let rejected = crate::answer_verifier::AnswerVerifierOut {
        pass: false,
        missing_evidence_fields: vec!["output_format".to_string()],
        answer_incomplete_reason: "candidate still violates the requested shape".to_string(),
        should_retry: true,
        retry_instruction: "rewrite the terminal answer".to_string(),
        confidence: 0.95,
    };

    assert!(retry_verifier_accepts_rewritten_answer(&accepted));
    assert!(!retry_verifier_accepts_rewritten_answer(&rejected));
}

#[test]
fn verifier_retry_commit_replaces_stale_visible_reply() {
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-verifier-retry-commit", "ask", "prompt");
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["output_format".to_string()],
        answer_incomplete_reason: "candidate omitted the requested terminal shape".to_string(),
        should_retry: true,
        retry_instruction: "rewrite the final answer from observed evidence".to_string(),
        confidence: 0.96,
    });
    let mut reply = AskReply::non_llm("stale raw observation".to_string())
        .with_messages(vec!["stale raw observation".to_string()])
        .with_task_journal(journal);

    commit_answer_verifier_retry_answer(&mut reply, "grounded rewritten answer".to_string());

    assert_eq!(reply.text, "grounded rewritten answer");
    assert_eq!(
        reply.messages,
        vec!["grounded rewritten answer".to_string()]
    );
    assert!(!reply.should_fail_task);
    assert!(reply.error_text.is_none());
    assert!(reply.is_llm_reply);
    let journal = reply.task_journal.as_ref().expect("journal");
    assert!(journal.answer_verifier_summary.is_none());
    assert_eq!(
        journal.final_answer.as_deref(),
        Some("grounded rewritten answer")
    );
    assert_eq!(
        journal.final_status,
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
    assert_eq!(
        journal.final_stop_signal.as_deref(),
        Some(crate::task_journal::ANSWER_VERIFIER_RECOVERED_TERMINAL_STOP_SIGNAL)
    );
}

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
fn answer_verifier_retry_summary_allows_recoverable_verifier_failure_reply() {
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Failure);
    journal.final_failure_attribution = Some("contract_gap".to_string());
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["output_format".to_string()],
        answer_incomplete_reason: "field label instead of clear final answer".to_string(),
        should_retry: true,
        retry_instruction: "rewrite from observed machine state".to_string(),
        confidence: 0.62,
    });
    let mut reply =
        AskReply::non_llm("approval_pending_task".to_string()).with_task_journal(journal);
    reply.should_fail_task = true;

    let summary = answer_verifier_retry_summary(&reply, None).expect("recoverable verifier gap");

    assert_eq!(summary.missing_evidence_fields, vec!["output_format"]);
}

#[test]
fn answer_verifier_retry_summary_allows_preterminal_should_fail_reply() {
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
    journal.final_failure_attribution = Some("contract_gap".to_string());
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["output_format".to_string()],
        answer_incomplete_reason: "candidate needs a corrected terminal shape".to_string(),
        should_retry: true,
        retry_instruction: "rewrite using the requested terminal contract".to_string(),
        confidence: 0.9,
    });
    let mut reply = AskReply::non_llm("field_label_only".to_string()).with_task_journal(journal);
    reply.should_fail_task = true;

    let summary = answer_verifier_retry_summary(&reply, None).expect("preterminal retry gap");

    assert_eq!(summary.missing_evidence_fields, vec!["output_format"]);
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
fn answer_verifier_retry_summary_skips_confirmed_missing_file_delivery() {
    let mut route = route_result(OutputResponseShape::FileToken);
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = OutputDeliveryIntent::FileSingle;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-missing-file-delivery", "ask", "prompt");
    journal.push_step_result(&ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"find_name","count":0,"results":[],"root":"/workspace","pattern":"definitely_missing_named_file_rustclaw_001.txt"},"text":"{}"}"#,
    ));
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["path".to_string(), "content_excerpt".to_string()],
        answer_incomplete_reason: "file delivery target is confirmed missing".to_string(),
        should_retry: true,
        retry_instruction: "repeat missing file search".to_string(),
        confidence: 0.95,
    });
    let mut reply = AskReply::non_llm(
        "没找到 definitely_missing_named_file_rustclaw_001.txt 这个文件。".to_string(),
    )
    .with_task_journal(journal);

    assert!(answer_verifier_retry_summary(&reply, Some(&route)).is_none());
    assert!(
        suppress_answer_verifier_retry_if_confirmed_missing_file_delivery(&mut reply, Some(&route))
    );
    assert!(reply
        .task_journal
        .as_ref()
        .and_then(|journal| journal.answer_verifier_summary.as_ref())
        .is_none());
}

#[test]
fn confirmed_missing_file_delivery_suppresses_retry_without_legacy_delivery_intent() {
    let mut route = route_result(OutputResponseShape::FileToken);
    route.wants_file_delivery = true;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = OutputDeliveryIntent::None;
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-missing-file-delivery-no-intent",
        "ask",
        "prompt",
    );
    journal.push_step_result(&ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"find_name","count":0,"exact":false,"patterns":["definitely_missing_named_file_golden_001.txt"],"results":[],"root":""},"text":"{\"action\":\"find_name\",\"count\":0,\"exact\":false,\"patterns\":[\"definitely_missing_named_file_golden_001.txt\"],\"results\":[],\"root\":\"\"}"}"#,
    ));
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["path".to_string()],
        answer_incomplete_reason: "file delivery target is confirmed missing".to_string(),
        should_retry: true,
        retry_instruction: "repeat missing file search".to_string(),
        confidence: 0.9,
    });
    let mut reply = AskReply::non_llm("definitely_missing_named_file_golden_001.txt".to_string())
        .with_task_journal(journal);

    assert!(answer_verifier_retry_summary(&reply, Some(&route)).is_none());
    assert!(
        suppress_answer_verifier_retry_if_confirmed_missing_file_delivery(&mut reply, Some(&route))
    );
    assert!(reply
        .task_journal
        .as_ref()
        .and_then(|journal| journal.answer_verifier_summary.as_ref())
        .is_none());
}

#[test]
fn confirmed_missing_file_delivery_does_not_suppress_success_token_claim() {
    let mut route = route_result(OutputResponseShape::FileToken);
    route.wants_file_delivery = true;
    route.output_contract.delivery_required = true;
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-missing-file-delivery-token-claim",
        "ask",
        "prompt",
    );
    journal.push_step_result(&ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"find_name","count":0,"results":[],"root":""},"text":"{}"}"#,
    ));
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["path".to_string()],
        answer_incomplete_reason: "file delivery target is confirmed missing".to_string(),
        should_retry: true,
        retry_instruction: "repeat missing file search".to_string(),
        confidence: 0.9,
    });
    let mut reply = AskReply::non_llm("FILE:/tmp/definitely_missing_named_file.txt".to_string())
        .with_task_journal(journal);

    assert!(answer_verifier_retry_summary(&reply, Some(&route)).is_some());
    assert!(
        !suppress_answer_verifier_retry_if_confirmed_missing_file_delivery(
            &mut reply,
            Some(&route)
        )
    );
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
fn terminal_model_answer_suppresses_output_format_only_verifier_retry() {
    let answer = "RustClaw combines the local clawd runtime, channel entry points, and skill dispatch into one deployable stack.";
    let mut route = route_result(OutputResponseShape::Free);
    route.output_contract.locator_kind = OutputLocatorKind::None;
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal.push_step_result(&ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"read_range","excerpt":"RustClaw runtime overview","path":"README.md"},"text":"RustClaw runtime overview"}"#,
    ));
    journal.push_step_result(&ok_step("step_2", "synthesize_answer", answer));
    journal.record_finalizer_summary(crate::task_journal::TaskJournalFinalizerSummary {
        stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
        disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
        contract_ok: true,
        used_evidence_ids_count: 1,
        ..Default::default()
    });
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["output_format".to_string()],
        answer_incomplete_reason: "terminal answer shape mismatch".to_string(),
        should_retry: true,
        retry_instruction: "rewrite terminal answer".to_string(),
        confidence: 0.9,
    });
    let mut reply = AskReply::non_llm(answer.to_string())
        .with_messages(vec![answer.to_string()])
        .with_task_journal(journal);

    assert!(suppress_answer_verifier_retry_if_structurally_satisfied(
        &mut reply,
        Some(&route)
    ));
    assert!(answer_verifier_retry_summary(&reply, Some(&route)).is_none());
}

#[test]
fn terminal_model_answer_does_not_suppress_non_format_evidence_gap() {
    let answer = "RustClaw combines the local clawd runtime, channel entry points, and skill dispatch into one deployable stack.";
    let mut route = route_result(OutputResponseShape::Free);
    route.output_contract.locator_kind = OutputLocatorKind::None;
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal.push_step_result(&ok_step("step_1", "synthesize_answer", answer));
    journal.record_finalizer_summary(crate::task_journal::TaskJournalFinalizerSummary {
        stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
        disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
        contract_ok: true,
        used_evidence_ids_count: 1,
        ..Default::default()
    });
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["output_format".to_string(), "content_excerpt".to_string()],
        answer_incomplete_reason: "content evidence is still missing".to_string(),
        should_retry: true,
        retry_instruction: "collect content evidence".to_string(),
        confidence: 0.9,
    });
    let mut reply = AskReply::non_llm(answer.to_string())
        .with_messages(vec![answer.to_string()])
        .with_task_journal(journal);

    assert!(!suppress_answer_verifier_retry_if_structurally_satisfied(
        &mut reply,
        Some(&route)
    ));
    assert!(answer_verifier_retry_summary(&reply, Some(&route)).is_some());
}

#[test]
fn terminal_model_answer_replaces_raw_observation_before_verifier() {
    let raw_readme = "# RustClaw\n\nRustClaw is a local Rust agent runtime centered on `clawd`.";
    let answer = "RustClaw 是以 `clawd` 为核心的本地 Rust 智能体运行时。它整合多渠道聊天、任务执行、工具和技能路由等能力。它面向通过聊天应用或浏览器完成日常使用和管理。";
    let mut route = route_result(OutputResponseShape::Strict);
    route.output_contract.exact_sentence_count = Some(3);
    route.output_contract.locator_kind = OutputLocatorKind::None;
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    let read_step_output = json!({
        "extra": {
            "action": "read_range",
            "excerpt": raw_readme,
            "path": "README.md",
        },
        "text": raw_readme,
    })
    .to_string();
    journal.push_step_result(&ok_step("step_1", "fs_basic", &read_step_output));
    journal.push_step_result(&ok_step("step_2", "respond", answer));
    journal.record_finalizer_summary(crate::task_journal::TaskJournalFinalizerSummary {
        stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
        disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
        contract_ok: true,
        used_evidence_ids_count: 1,
        ..Default::default()
    });
    let mut reply = AskReply::non_llm(raw_readme.to_string())
        .with_messages(vec![raw_readme.to_string()])
        .with_task_journal(journal);

    assert!(prefer_terminal_model_answer_for_verifier_candidate(
        &mut reply,
        Some(&route)
    ));
    assert_eq!(reply.text, answer);
}

#[test]
fn terminal_model_answer_does_not_replace_richer_machine_projection_with_observed_scalar() {
    let mut route = route_result(OutputResponseShape::Free);
    route.output_contract.locator_kind = OutputLocatorKind::None;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-service-status-terminal", "ask", "status");
    let service_output = json!({
        "extra": {
            "manager_type": "rustclaw",
            "post_state": "telegramd=running",
            "pre_state": "telegramd=running",
            "service_name": "telegramd",
            "status": "ok",
            "summary": "Status: telegramd=running",
            "target": "telegramd",
            "verified": true
        },
        "text": "Status: telegramd=running"
    })
    .to_string();
    journal.push_step_result(&ok_step("step_1", "service_control", &service_output));
    journal.push_step_result(&ok_step("step_2", "respond", "Status: telegramd=running"));
    journal.record_finalizer_summary(crate::task_journal::TaskJournalFinalizerSummary {
        stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
        disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
        contract_ok: true,
        used_evidence_ids_count: 1,
        ..Default::default()
    });
    let observed_projection = "target=telegramd service_name=telegramd post_state=telegramd=running pre_state=telegramd=running status=ok verified=true manager_type=rustclaw source=service_control";
    let mut reply = AskReply::non_llm(observed_projection.to_string())
        .with_messages(vec![observed_projection.to_string()])
        .with_task_journal(journal);

    assert!(!prefer_terminal_model_answer_for_verifier_candidate(
        &mut reply,
        Some(&route)
    ));
    assert_eq!(reply.text, observed_projection);
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

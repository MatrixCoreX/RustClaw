use super::{
    answer_verifier_retry_summary, evaluate_round_outcome, initial_execution_recipe_spec,
    mark_reply_failed_after_answer_verifier_exhausted, parse_log_analyze_finding,
    should_stop_for_observed_finalize, suppress_answer_verifier_retry_if_structurally_satisfied,
    try_recover_content_excerpt_summary_answer_verifier_gap,
    try_recover_generic_path_content_read_range_answer_verifier_gap,
    try_recover_log_analyze_answer_verifier_gap, try_recover_structured_count_answer_verifier_gap,
    try_recover_structured_search_answer_verifier_gap, AgentLoopGuardPolicy, RoundOutcome,
};
use crate::{
    agent_engine::{AgentRunContext, LoopState},
    execution_recipe::{
        ExecutionRecipeKind, ExecutionRecipeProfile, ExecutionRecipeRuntimeState,
        ExecutionRecipeSpec, ExecutionRecipeTargetScope,
    },
    executor::{StepExecutionResult, StepExecutionStatus},
    AgentAction, AskReply, ClaimedTask, IntentOutputContract, OutputDeliveryIntent,
    OutputLocatorKind, OutputResponseShape, OutputSemanticKind, ResumeBehavior, RiskCeiling,
    RouteResult, ScheduleKind,
};
use serde_json::json;

fn route_result(shape: OutputResponseShape) -> RouteResult {
    RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "test".to_string(),
        needs_clarify: false,
        route_reason: String::new(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: shape,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: Default::default(),
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    }
}

fn ok_step(step_id: &str, skill: &str, output: &str) -> StepExecutionResult {
    StepExecutionResult {
        step_id: step_id.to_string(),
        skill: skill.to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(output.to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    }
}

fn test_task() -> ClaimedTask {
    ClaimedTask {
        task_id: "task-loop-control".to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "telegram".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    }
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

#[test]
fn parses_truncated_log_analyze_output_prefix() {
    let finding = parse_log_analyze_finding(
        r#"{"keyword_counts":{"error":115,"failed":48,"panic":23,"timeout":26,"warn":72},"path":"/tmp/logs/clawd.run.log","recent_matches":["... ...(truncated)""#,
    )
    .expect("truncated prefix still contains counts and path");

    assert_eq!(finding.path, "/tmp/logs/clawd.run.log");
    assert_eq!(finding.total_hits, 284);
    assert_eq!(finding.keyword_counts[0], ("error".to_string(), 115));
}

#[test]
fn log_analyze_verifier_exhaustion_recovers_with_structural_summary() {
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["clawd.run.log".to_string()],
        answer_incomplete_reason: "candidate omitted clawd.run.log counts".to_string(),
        should_retry: true,
        retry_instruction: "include every analyzed log".to_string(),
        confidence: 0.95,
    });
    journal.step_results.push(crate::task_journal::TaskJournalStepTrace {
        step_id: "step_1".to_string(),
        skill: "log_analyze".to_string(),
        status: StepExecutionStatus::Ok,
        output_excerpt: Some(
            r#"{"keyword_counts":{"error":1286,"failed":939,"timeout":308},"path":"/logs/model_io.log.2026-05-13","recent_matches":[]}"#
                .to_string(),
        ),
        error_excerpt: None,
        started_at: 0,
        finished_at: 0,
    });
    journal.step_results.push(crate::task_journal::TaskJournalStepTrace {
        step_id: "step_2".to_string(),
        skill: "log_analyze".to_string(),
        status: StepExecutionStatus::Ok,
        output_excerpt: Some(
            r#"{"keyword_counts":{"error":115,"warn":72,"failed":48},"path":"/logs/clawd.run.log","recent_matches":["...(truncated)"]}"#
                .to_string(),
        ),
        error_excerpt: None,
        started_at: 0,
        finished_at: 0,
    });
    let mut reply = AskReply::non_llm("partial answer".to_string())
        .with_messages(vec![
            "**执行过程**\n1. 调用技能 `log_analyze`。".to_string(),
            "partial answer".to_string(),
        ])
        .with_task_journal(journal);

    assert!(try_recover_log_analyze_answer_verifier_gap(
        "快速看一下 logs 目录里最近最值得注意的错误或异常",
        &mut reply
    ));

    assert!(!reply.should_fail_task);
    assert!(reply.text.contains("model_io.log.2026-05-13"));
    assert!(reply.text.contains("clawd.run.log"));
    assert!(reply.text.contains("error 115"));
    let journal = reply.task_journal.as_ref().expect("journal");
    assert_eq!(
        journal.final_status,
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
    assert!(journal.answer_verifier_summary.is_none());
}

#[test]
fn structured_search_verifier_exhaustion_recovers_with_full_candidate_list() {
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["candidates".to_string()],
        answer_incomplete_reason:
            "answer reports 1 README file but observed evidence shows 3 README files".to_string(),
        should_retry: true,
        retry_instruction: "answer from the full observed results array".to_string(),
        confidence: 0.94,
    });
    journal.step_results.push(crate::task_journal::TaskJournalStepTrace {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output_excerpt: Some(
            r#"{"action":"find_name","count":3,"patterns":["README"],"results":["README.md","UI/README.md","docs/README.md"],"root":"/repo"}"#
                .to_string(),
        ),
        error_excerpt: None,
        started_at: 0,
        finished_at: 0,
    });
    let mut reply = AskReply::non_llm("Found README.md.".to_string())
        .with_messages(vec![
            "**Execution**\n1. Ran tool `fs_basic`.".to_string(),
            "Found README.md.".to_string(),
        ])
        .with_task_journal(journal);
    let mut route = route_result(OutputResponseShape::Strict);
    route.output_contract.semantic_kind = OutputSemanticKind::FilePaths;

    assert!(try_recover_structured_search_answer_verifier_gap(
        Some(&route),
        "Find files named README under the current repo.",
        &mut reply
    ));

    assert!(!reply.should_fail_task);
    assert!(reply.text.contains("Found 3 candidates"));
    assert!(reply.text.contains("README.md"));
    assert!(reply.text.contains("UI/README.md"));
    assert!(reply.text.contains("docs/README.md"));
    let journal = reply.task_journal.as_ref().expect("journal");
    assert_eq!(
        journal.final_status,
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
    assert!(journal.answer_verifier_summary.is_none());
}

#[test]
fn structured_search_recovery_does_not_override_directory_purpose_summary() {
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["candidates".to_string()],
        answer_incomplete_reason: "answer used recursive candidates instead of direct entries"
            .to_string(),
        should_retry: true,
        retry_instruction: "answer from the direct list_dir evidence".to_string(),
        confidence: 0.94,
    });
    journal.step_results.push(crate::task_journal::TaskJournalStepTrace {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output_excerpt: Some(
            r#"{"action":"find_ext","count":50,"ext":"toml","results":["Cargo.toml","configs/config.toml"],"root":"/repo"}"#
                .to_string(),
        ),
        error_excerpt: None,
        started_at: 0,
        finished_at: 0,
    });
    let mut reply =
        AskReply::non_llm("Found 50 candidates.".to_string()).with_task_journal(journal);
    let mut route = route_result(OutputResponseShape::Strict);
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryPurposeSummary;

    assert!(!try_recover_structured_search_answer_verifier_gap(
        Some(&route),
        "List top-level toml files and explain them briefly.",
        &mut reply
    ));
    assert!(!reply.should_fail_task);
    assert_eq!(reply.text, "Found 50 candidates.");
    assert!(reply
        .task_journal
        .as_ref()
        .and_then(|journal| journal.answer_verifier_summary.as_ref())
        .is_some());
}

#[test]
fn structured_count_verifier_exhaustion_recovers_with_count_inventory() {
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["count".to_string()],
        answer_incomplete_reason: "answer asks to rerun instead of delivering observed count"
            .to_string(),
        should_retry: true,
        retry_instruction: "use the observed counts.total field".to_string(),
        confidence: 0.94,
    });
    journal.step_results.push(crate::task_journal::TaskJournalStepTrace {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output_excerpt: Some(
            r#"{"action":"count_inventory","counts":{"dirs":6,"files":58,"hidden":0,"total":64},"path":"/repo/scripts","recursive":false}"#
                .to_string(),
        ),
        error_excerpt: None,
        started_at: 0,
        finished_at: 0,
    });
    let mut route = route_result(OutputResponseShape::OneSentence);
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
    let mut reply = AskReply::non_llm("需要重新触发计数任务。".to_string())
        .with_messages(vec![
            "**执行过程**\n1. 调用工具 `fs_basic`。".to_string(),
            "需要重新触发计数任务。".to_string(),
        ])
        .with_task_journal(journal);

    assert!(try_recover_structured_count_answer_verifier_gap(
        Some(&route),
        "先数一下 scripts 目录直接有多少个子项",
        &mut reply
    ));

    assert!(!reply.should_fail_task);
    assert!(reply.text.contains("64"));
    assert!(reply.text.contains("58"));
    assert!(reply.text.contains("6"));
    let journal = reply.task_journal.as_ref().expect("journal");
    assert_eq!(
        journal.final_status,
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
    assert!(journal.answer_verifier_summary.is_none());
}

#[test]
fn content_excerpt_summary_verifier_exhaustion_recovers_with_synthesis_output() {
    let mut route = route_result(OutputResponseShape::Free);
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal.record_route_result(&route);
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["content_excerpt".to_string()],
        answer_incomplete_reason: "final answer dropped synthesized summary".to_string(),
        should_retry: true,
        retry_instruction: "use the full synthesized summary".to_string(),
        confidence: 0.94,
    });
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_0",
            "fs_basic",
            r#"{"action":"read_range","path":"README.md","excerpt":"1|# RustClaw\n2|Observed excerpt for summary."}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_1".to_string(),
            skill: "synthesize_answer".to_string(),
            status: StepExecutionStatus::Ok,
            output_excerpt: Some(
                "Summary from observed excerpt covering all required facts.".to_string(),
            ),
            error_excerpt: None,
            started_at: 0,
            finished_at: 0,
        });
    let mut reply = AskReply::non_llm("partial answer".to_string())
        .with_messages(vec![
            "**Execution**\n1. Read file excerpt.".to_string(),
            "partial answer".to_string(),
        ])
        .with_task_journal(journal);

    assert!(try_recover_content_excerpt_summary_answer_verifier_gap(
        Some(&route),
        &mut reply
    ));

    assert!(!reply.should_fail_task);
    assert_eq!(
        reply.text,
        "Summary from observed excerpt covering all required facts."
    );
    assert_eq!(
        reply.messages,
        vec!["Summary from observed excerpt covering all required facts.".to_string()]
    );
    let journal = reply.task_journal.as_ref().expect("journal");
    assert_eq!(
        journal.final_status,
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
    assert!(journal.answer_verifier_summary.is_none());
}

#[test]
fn workspace_project_summary_verifier_exhaustion_recovers_with_synthesis_output() {
    let mut route = route_result(OutputResponseShape::Free);
    route.output_contract.semantic_kind = OutputSemanticKind::WorkspaceProjectSummary;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    let mut journal = crate::task_journal::TaskJournal::for_task("task-workspace", "ask", "prompt");
    journal.record_route_result(&route);
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["content_excerpt".to_string()],
        answer_incomplete_reason: "retry exhausted after an exploratory miss".to_string(),
        should_retry: true,
        retry_instruction: "answer from the already observed README excerpt".to_string(),
        confidence: 0.95,
    });
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            r#"{"action":"read_range","path":"README.md","excerpt":"15|- multi-channel entry points: Telegram, WeChat, Feishu, Lark, WhatsApp Cloud, WhatsApp Web, browser UI, and optional `webd`"}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "synthesize_answer",
            "RustClaw supports multi-channel entry via Telegram, WeChat, Feishu, Lark, WhatsApp Cloud, WhatsApp Web, browser UI, and optional `webd`. Concrete channel setup depends on the chosen channel's documented setup path.",
        ));
    let mut reply =
        AskReply::non_llm("failed exploratory answer".to_string()).with_task_journal(journal);

    assert!(try_recover_content_excerpt_summary_answer_verifier_gap(
        Some(&route),
        &mut reply
    ));

    assert!(!reply.should_fail_task);
    assert!(reply.text.contains("multi-channel entry"));
    assert_eq!(reply.messages, vec![reply.text.clone()]);
    let journal = reply.task_journal.as_ref().expect("journal");
    assert_eq!(
        journal.final_status,
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
    assert!(journal.answer_verifier_summary.is_none());
}

#[test]
fn workspace_project_summary_verifier_exhaustion_does_not_recover_unsupported_claims() {
    let mut route = route_result(OutputResponseShape::Free);
    route.output_contract.semantic_kind = OutputSemanticKind::WorkspaceProjectSummary;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    let mut journal = crate::task_journal::TaskJournal::for_task("task-workspace", "ask", "prompt");
    journal.record_route_result(&route);
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["unsupported_claims".to_string()],
        answer_incomplete_reason: "answer adds setup steps not supported by observed excerpts"
            .to_string(),
        should_retry: true,
        retry_instruction: "rewrite from observed channel surfaces only".to_string(),
        confidence: 0.95,
    });
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            r#"{"action":"read_range","path":"README.md","excerpt":"15|- multi-channel entry points: Telegram, WeChat, Feishu, Lark, WhatsApp Cloud, WhatsApp Web, browser UI, and optional `webd`"}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "synthesize_answer",
            "Unsupported setup steps should not be recovered.",
        ));
    let mut reply =
        AskReply::non_llm("failed exploratory answer".to_string()).with_task_journal(journal);

    assert!(!try_recover_content_excerpt_summary_answer_verifier_gap(
        Some(&route),
        &mut reply
    ));
}

#[test]
fn generic_path_content_verifier_exhaustion_recovers_with_read_range_excerpt() {
    let route = route_result(OutputResponseShape::Free);
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-read-range", "ask", "tail log");
    journal.record_route_result(&route);
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["content_excerpt".to_string(), "path".to_string()],
        answer_incomplete_reason: "answer did not include read_range fields".to_string(),
        should_retry: true,
        retry_instruction: "include path and content_excerpt".to_string(),
        confidence: 0.5,
    });
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_1".to_string(),
            skill: "fs_basic".to_string(),
            status: StepExecutionStatus::Ok,
            output_excerpt: Some(
                json!({
                    "action": "read_range",
                    "mode": "tail",
                    "requested_n": 2,
                    "path": "logs/clawd.log",
                    "resolved_path": "/repo/logs/clawd.log",
                    "excerpt": "41|first log line\n42|second log line"
                })
                .to_string(),
            ),
            error_excerpt: None,
            started_at: 0,
            finished_at: 0,
        });
    let mut reply = AskReply::non_llm("partial answer".to_string())
        .with_messages(vec![
            "**Execution**\n1. Read the file range.".to_string(),
            "partial answer".to_string(),
        ])
        .with_task_journal(journal);

    assert!(
        try_recover_generic_path_content_read_range_answer_verifier_gap(Some(&route), &mut reply)
    );

    assert!(!reply.should_fail_task);
    assert_eq!(reply.text, "first log line\nsecond log line");
    assert_eq!(reply.messages, vec!["first log line\nsecond log line"]);
    let journal = reply.task_journal.as_ref().expect("journal");
    assert_eq!(
        journal.final_status,
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
    assert!(journal.answer_verifier_summary.is_none());
}

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
    assert!(reply.text.contains("Verification issue"));
    let journal = reply.task_journal.as_ref().expect("journal");
    assert_eq!(
        journal.final_status,
        Some(crate::task_journal::TaskJournalFinalStatus::Failure)
    );
    assert_eq!(journal.final_answer.as_deref(), Some(reply.text.as_str()));
}

fn test_policy() -> AgentLoopGuardPolicy {
    AgentLoopGuardPolicy {
        max_steps: 8,
        max_rounds: 4,
        recoverable_failure_extra_rounds: 1,
        repeat_action_limit: 3,
        no_progress_limit: 1,
        multi_round_enabled: true,
        answer_verifier_retry_limit: 2,
        ops_closed_loop: Default::default(),
    }
}

#[test]
fn observed_scalar_output_can_stop_loop_without_second_round() {
    let mut loop_state = LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "system_basic",
        r#"{"action":"extract_field","exists":true,"field_path":"name","value_text":"rustclaw","value":"rustclaw","value_type":"string"}"#,
    ));
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({"action":"extract_field"}),
    }];
    assert!(should_stop_for_observed_finalize(
        Some(&AgentRunContext {
            route_result: Some(route_result(OutputResponseShape::Scalar)),
            ..Default::default()
        }),
        &loop_state,
        &actions,
    ));
}

#[test]
fn observed_config_basic_scalar_output_can_stop_loop_without_second_round() {
    let mut loop_state = LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "config_basic",
        r#"{"action":"extract_field","exists":true,"field_path":"run_cmd.planner_kind","value_text":"tool","value":"tool","value_type":"string"}"#,
    ));
    let actions = vec![AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args: json!({"action":"read_field","path":"configs/skills_registry.toml","field_path":"run_cmd.planner_kind"}),
    }];
    assert!(should_stop_for_observed_finalize(
        Some(&AgentRunContext {
            route_result: Some(route_result(OutputResponseShape::Strict)),
            ..Default::default()
        }),
        &loop_state,
        &actions,
    ));
}

#[test]
fn observation_only_freeform_round_can_stop_for_observed_fallback() {
    let mut loop_state = LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "list_dir",
        "README.md\ndocs/\ncrates/\n",
    ));
    let actions = vec![AgentAction::CallSkill {
        skill: "list_dir".to_string(),
        args: json!({"path":"."}),
    }];
    assert!(should_stop_for_observed_finalize(
        Some(&AgentRunContext {
            route_result: Some(route_result(OutputResponseShape::Free)),
            ..Default::default()
        }),
        &loop_state,
        &actions,
    ));
}

#[test]
fn unscoped_workspace_evidence_drafting_does_not_stop_on_search_only() {
    let mut loop_state = LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_search",
        r#"{"action":"find_name","count":2,"results":["README.md","USAGE.md"]}"#,
    ));
    let mut route = route_result(OutputResponseShape::Free);
    route.resolved_intent =
        "Write a short setup note grounded in the current workspace docs".to_string();
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    let actions = vec![AgentAction::CallSkill {
        skill: "fs_search".to_string(),
        args: json!({"action":"find_name","pattern":"README"}),
    }];
    assert!(!should_stop_for_observed_finalize(
        Some(&AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        }),
        &loop_state,
        &actions,
    ));
}

#[test]
fn unscoped_workspace_evidence_drafting_can_stop_after_doc_read() {
    let mut loop_state = LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "system_basic",
        r#"{"action":"read_range","path":"README.md","excerpt":"1|# RustClaw\n2|## Setup"}"#,
    ));
    let mut route = route_result(OutputResponseShape::Free);
    route.resolved_intent =
        "Write a short setup note grounded in the current workspace docs".to_string();
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({"action":"read_range","path":"README.md","mode":"head","n":120}),
    }];
    assert!(should_stop_for_observed_finalize(
        Some(&AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        }),
        &loop_state,
        &actions,
    ));
}

#[test]
fn hidden_entries_scalar_output_can_stop_before_synthesis_followup() {
    let mut loop_state = LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "list_dir",
        ".git\nREADME.md\n.env\nsrc\n",
    ));
    let mut route = route_result(OutputResponseShape::Scalar);
    route.resolved_intent = "检查当前目录有没有隐藏文件，只回答有或没有，并补 3 个例子".to_string();
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::HiddenEntriesCheck;
    route.output_contract.locator_hint = ".".to_string();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "list_dir".to_string(),
            args: json!({"path":"."}),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
    ];
    assert!(should_stop_for_observed_finalize(
        Some(&AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        }),
        &loop_state,
        &actions,
    ));
}

#[test]
fn fs_basic_inventory_names_can_stop_before_synthesis_followup() {
    let mut loop_state = LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","path":"/tmp/document","resolved_path":"/tmp/document","files_only":true,"names_only":true,"names":["a.txt","b.md","c.png"]}"#,
    ));
    let mut route = route_result(OutputResponseShape::Free);
    route.ask_mode = crate::AskMode::planner_execute_plain();
    route.resolved_intent = "List file names from a known directory.".to_string();
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::FileNames;
    route.output_contract.locator_hint = "document".to_string();
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({"action":"list_dir","path":"/tmp/document","files_only":true,"names_only":true}),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
    ];
    assert!(should_stop_for_observed_finalize(
        Some(&AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        }),
        &loop_state,
        &actions,
    ));
}

#[test]
fn existence_with_path_free_output_can_stop_before_second_round() {
    let mut loop_state = LoopState::new(2);
    loop_state.round_no = 1;
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "system_basic",
        r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":true,"fact":{"kind":"file","path":"rustclaw.service","resolved_path":"/home/guagua/rustclaw/rustclaw.service","size_bytes":1190},"path":"/home/guagua/rustclaw/rustclaw.service"}],"include_missing":true}"#,
    ));
    let mut route = route_result(OutputResponseShape::Free);
    route.resolved_intent =
        "检查仓库里有没有 rustclaw.service，只回答有或没有，并给出路径".to_string();
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_hint = "rustclaw.service".to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({"action":"path_batch_facts","paths":["/home/guagua/rustclaw/rustclaw.service"]}),
    }];
    assert!(should_stop_for_observed_finalize(
        Some(&AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        }),
        &loop_state,
        &actions,
    ));
}

#[test]
fn missing_path_batch_facts_existence_contract_stops_before_second_round() {
    let mut loop_state = LoopState::new(2);
    loop_state.round_no = 1;
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "system_basic",
        r#"{"action":"path_batch_facts","count":1,"facts":[{"error":"not found","exists":false,"path":"plan/missing.md"}],"include_missing":true}"#,
    ));
    let mut route = route_result(OutputResponseShape::Free);
    route.resolved_intent =
        "Read plan/missing.md; if it is absent, search plan for related markdown files".to_string();
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_hint = "plan/missing.md".to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({"action":"path_batch_facts","paths":["plan/missing.md"]}),
    }];
    assert!(should_stop_for_observed_finalize(
        Some(&AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        }),
        &loop_state,
        &actions,
    ));
}

#[test]
fn missing_path_batch_facts_content_contract_continues_for_possible_fallback() {
    let mut loop_state = LoopState::new(2);
    loop_state.round_no = 1;
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "system_basic",
        r#"{"action":"path_batch_facts","count":1,"facts":[{"error":"not found","exists":false,"path":"plan/missing.md"}],"include_missing":true}"#,
    ));
    let mut route = route_result(OutputResponseShape::Free);
    route.resolved_intent =
        "Read plan/missing.md; if it is absent, search plan for related markdown files".to_string();
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_hint = "plan/missing.md".to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({"action":"path_batch_facts","paths":["plan/missing.md"]}),
    }];
    assert!(!should_stop_for_observed_finalize(
        Some(&AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        }),
        &loop_state,
        &actions,
    ));
}

#[test]
fn structured_keys_free_output_can_stop_before_second_round() {
    let mut loop_state = LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "system_basic",
        r#"{"action":"structured_keys","path":"/tmp/package.json","resolved_path":"/tmp/package.json","field_path":"scripts","exists":true,"container_type":"object","count":3,"keys":["build","dev","lint"]}"#,
    ));
    let mut route = route_result(OutputResponseShape::Free);
    route.route_reason = "llm_contract:generic_explicit_path_structured_keys".to_string();
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/package.json".to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({"action":"structured_keys","path":"/tmp/package.json","field_path":"scripts"}),
    }];
    assert!(should_stop_for_observed_finalize(
        Some(&AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        }),
        &loop_state,
        &actions,
    ));
}

#[test]
fn extract_fields_free_output_can_stop_before_second_round() {
    let mut loop_state = LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "system_basic",
        r#"{"action":"extract_fields","path":"/tmp/config.toml","resolved_path":"/tmp/config.toml","count":2,"results":[{"field_path":"database.sqlite_path","exists":true,"value_type":"string","value_text":"data/rustclaw.db","value":"data/rustclaw.db"},{"field_path":"tools.allow_sudo","exists":true,"value_type":"bool","value_text":"true","value":true}]}"#,
    ));
    let mut route = route_result(OutputResponseShape::Free);
    route.route_reason = "llm_contract:generic_explicit_path_extract_fields".to_string();
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/config.toml".to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({"action":"extract_fields","path":"/tmp/config.toml","field_paths":["database.sqlite_path","tools.allow_sudo"]}),
    }];
    assert!(should_stop_for_observed_finalize(
        Some(&AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        }),
        &loop_state,
        &actions,
    ));
}

#[test]
fn health_check_scalar_summary_continues_to_synthesis() {
    let mut loop_state = LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "health_check",
        r#"{"clawd_process_count":1,"telegramd_process_count":0,"clawd_health_port_open":true,"clawd_log":{"exists":true,"keyword_error_count":0},"telegramd_log":{"exists":false},"system_health":{"os_family":"macos","warnings":[]}}"#,
    ));
    let mut route = route_result(OutputResponseShape::Scalar);
    route.resolved_intent =
        "执行基础健康检查，仅提取并返回操作系统相关的关键字段，排除 RustClaw 自身的状态摘要"
            .to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "health_check".to_string(),
        args: json!({}),
    }];
    assert!(!should_stop_for_observed_finalize(
        Some(&AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        }),
        &loop_state,
        &actions,
    ));
}

#[test]
fn recipe_waiting_for_validation_does_not_stop_on_observed_output() {
    let mut loop_state = LoopState::new(2);
    loop_state.execution_recipe = ExecutionRecipeRuntimeState {
        kind: ExecutionRecipeKind::OpsClosedLoop,
        validation_required: true,
        saw_mutation: true,
        ..Default::default()
    };
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "run_cmd", "configuration updated\n"));
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command":"cat ./config.json"}),
    }];
    assert!(!should_stop_for_observed_finalize(
        Some(&AgentRunContext {
            route_result: Some(route_result(OutputResponseShape::Free)),
            ..Default::default()
        }),
        &loop_state,
        &actions,
    ));
}

#[test]
fn recipe_inspect_stage_does_not_stop_on_observed_output() {
    let mut loop_state = LoopState::new(2);
    loop_state.execution_recipe = ExecutionRecipeRuntimeState {
        kind: ExecutionRecipeKind::OpsClosedLoop,
        phase: crate::execution_recipe::ExecutionRecipePhase::Inspect,
        inspect_first: true,
        validation_required: true,
        ..Default::default()
    };
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "list_dir", "index.html\n"));
    let actions = vec![AgentAction::CallSkill {
        skill: "list_dir".to_string(),
        args: json!({"path":"document/nl_ops_http_demo"}),
    }];
    assert!(!should_stop_for_observed_finalize(
        Some(&AgentRunContext {
            route_result: Some(route_result(OutputResponseShape::Scalar)),
            ..Default::default()
        }),
        &loop_state,
        &actions,
    ));
}

#[test]
fn recipe_done_does_not_scan_user_text_for_success_marker() {
    let mut loop_state = LoopState::new(2);
    loop_state.execution_recipe = ExecutionRecipeRuntimeState {
        kind: ExecutionRecipeKind::OpsClosedLoop,
        phase: crate::execution_recipe::ExecutionRecipePhase::Done,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        saw_mutation: true,
        saw_validation: true,
        ..Default::default()
    };
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "run_cmd", "ops-demo-ok\n"));
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command":"curl -s http://127.0.0.1:52752/ | grep -o ops-demo-ok"}),
    }];
    assert!(should_stop_for_observed_finalize(
        Some(&AgentRunContext {
            route_result: Some(route_result(OutputResponseShape::Scalar)),
            user_request: Some(
                "验证通过时请明确输出 VALIDATION_PASSED，然后直接结束。".to_string()
            ),
            ..Default::default()
        }),
        &loop_state,
        &actions,
    ));
}

#[test]
fn recoverable_recipe_failure_continues_next_round_and_keeps_repair_count() {
    let task = test_task();
    let policy = test_policy();
    let mut loop_state = LoopState::new(4);
    loop_state.round_no = 1;
    loop_state.execution_recipe = ExecutionRecipeRuntimeState {
        kind: ExecutionRecipeKind::OpsClosedLoop,
        phase: crate::execution_recipe::ExecutionRecipePhase::Repair,
        inspect_first: true,
        validation_required: true,
        max_repairs: 3,
        repair_count: 1,
        saw_inspect: true,
        saw_mutation: true,
        saw_validation: false,
        ..Default::default()
    };
    let outcome = RoundOutcome {
        executed_actions: 1,
        had_error: false,
        stop_signal: Some("recoverable_failure_continue_round".to_string()),
        next_goal_hint: Some("repair sing-box".to_string()),
        no_progress: false,
    };
    assert!(!evaluate_round_outcome(
        &task,
        &mut loop_state,
        &policy,
        &outcome
    ));
    assert_eq!(loop_state.execution_recipe.repair_count, 1);
    assert_eq!(
        loop_state.execution_recipe.phase,
        crate::execution_recipe::ExecutionRecipePhase::Repair
    );
    assert_eq!(loop_state.consecutive_no_progress, 0);
}

#[test]
fn recoverable_failure_at_round_cap_extends_loop_once() {
    let task = test_task();
    let mut policy = test_policy();
    policy.max_rounds = 2;
    policy.recoverable_failure_extra_rounds = 1;
    let mut loop_state = LoopState::new(2);
    loop_state.round_no = 2;
    let outcome = RoundOutcome {
        executed_actions: 1,
        had_error: false,
        stop_signal: Some("recoverable_failure_continue_round".to_string()),
        next_goal_hint: Some("try alternate locator".to_string()),
        no_progress: false,
    };

    assert!(!evaluate_round_outcome(
        &task,
        &mut loop_state,
        &policy,
        &outcome
    ));
    assert_eq!(loop_state.max_rounds, 3);
    assert_eq!(loop_state.recoverable_failure_extra_rounds_used, 1);
}

#[test]
fn recoverable_failure_extra_round_exhaustion_stops() {
    let task = test_task();
    let mut policy = test_policy();
    policy.max_rounds = 2;
    policy.recoverable_failure_extra_rounds = 1;
    let mut loop_state = LoopState::new(2);
    loop_state.round_no = 2;
    loop_state.recoverable_failure_extra_rounds_used = 1;
    let outcome = RoundOutcome {
        executed_actions: 1,
        had_error: false,
        stop_signal: Some("recoverable_failure_continue_round".to_string()),
        next_goal_hint: Some("try alternate locator".to_string()),
        no_progress: false,
    };

    assert!(evaluate_round_outcome(
        &task,
        &mut loop_state,
        &policy,
        &outcome
    ));
    assert_eq!(loop_state.max_rounds, 2);
    assert_eq!(loop_state.recoverable_failure_extra_rounds_used, 1);
}

#[test]
fn exhausted_recipe_budget_stops_next_round() {
    let task = test_task();
    let policy = test_policy();
    let mut loop_state = LoopState::new(4);
    loop_state.round_no = 2;
    loop_state.execution_recipe = ExecutionRecipeRuntimeState {
        kind: ExecutionRecipeKind::OpsClosedLoop,
        phase: crate::execution_recipe::ExecutionRecipePhase::Repair,
        inspect_first: true,
        validation_required: true,
        max_repairs: 2,
        repair_count: 3,
        saw_inspect: true,
        saw_mutation: true,
        saw_validation: false,
        ..Default::default()
    };
    let outcome = RoundOutcome {
        executed_actions: 1,
        had_error: false,
        stop_signal: Some("recipe_repair_budget_exhausted".to_string()),
        next_goal_hint: None,
        no_progress: false,
    };
    assert!(evaluate_round_outcome(
        &task,
        &mut loop_state,
        &policy,
        &outcome
    ));
    assert_eq!(loop_state.execution_recipe.repair_count, 3);
    assert_eq!(
        loop_state.execution_recipe.phase,
        crate::execution_recipe::ExecutionRecipePhase::Repair
    );
}

#[test]
fn explicit_execution_recipe_hint_takes_priority_over_local_detection() {
    let spec = initial_execution_recipe_spec(
        "configure sing-box and verify the proxy works",
        "configure sing-box and verify the proxy works",
        Some(&AgentRunContext {
            execution_recipe_hint: Some(ExecutionRecipeSpec {
                kind: ExecutionRecipeKind::OpsClosedLoop,
                profile: ExecutionRecipeProfile::CodeChange,
                target_scope: ExecutionRecipeTargetScope::Greenfield,
                inspect_first: true,
                validation_required: true,
                max_repairs: 2,
            }),
            route_result: Some(route_result(OutputResponseShape::Free)),
            user_request: Some("configure sing-box and verify the proxy works".to_string()),
            ..Default::default()
        }),
    );
    assert_eq!(spec.profile, ExecutionRecipeProfile::CodeChange);
    assert_eq!(spec.target_scope, ExecutionRecipeTargetScope::Greenfield);
}

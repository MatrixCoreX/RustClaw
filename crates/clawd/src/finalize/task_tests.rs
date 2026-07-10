use super::{
    answer_text_is_machine_json_payload, answer_verifier_failure_default_payload,
    answer_verifier_failure_machine_line, answer_verifier_failure_missing_fields_text,
    answer_verifier_failure_observed_facts, answer_verifier_forces_task_failure,
    answer_verifier_retry_applicable, answer_verifier_retry_observed_trace,
    answer_verifier_should_force_task_failure, apply_requested_machine_kv_summary_to_final_answer,
    ask_runtime_failure_machine_payload, assistant_memory_source_text,
    compose_answer_verifier_failure_user_message, delivery_path_gap_should_finalize_as_clarify,
    deterministic_config_guard_candidates_recovery,
    deterministic_content_tail_read_failure_recovery, deterministic_filtered_log_entry_recovery,
    deterministic_raw_tail_read_failure_recovery, deterministic_tree_summary_rows_failure_recovery,
    drop_execution_summaries_when_delivery_is_scalar, failed_task_lifecycle_payload,
    finalize_ask_checkpointed, journal_has_checkpointed_nonterminal_lifecycle,
    journal_has_missing_file_search_evidence, machine_payload_observed_facts,
    non_failure_final_status, normalize_existing_file_delivery_token_answer,
    record_answer_verifier_required_evidence_rollout_attribution,
    recover_requested_machine_kv_summary_final_answer, resume_context_has_directory_lookup_failure,
    resume_context_path_batch_facts_are_missing_only,
    resume_failure_is_unbound_path_lookup_clarify_result,
    should_reinsert_execution_summaries_for_delivery, should_use_answer_route_result,
};

use serde_json::json;

#[path = "task_tests/answer_verifier_recovery.rs"]
mod answer_verifier_recovery;
#[path = "task_tests/checkpoint_finalization.rs"]
mod checkpoint_finalization;
#[path = "task_tests/config_guard_recovery.rs"]
mod config_guard_recovery;
#[path = "task_tests/config_validation_delivery.rs"]
mod config_validation_delivery;
#[path = "task_tests/git_machine_kv_recovery.rs"]
mod git_machine_kv_recovery;
#[path = "task_tests/machine_kv_final_guard.rs"]
mod machine_kv_final_guard;
#[path = "task_tests/tree_summary_recovery.rs"]
mod tree_summary_recovery;

fn route_result(ask_mode: crate::AskMode) -> crate::RouteResult {
    crate::RouteResult {
        ask_mode,
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
    }
}

// ensure_journal_task_metrics_* tests 已搬移到 finalize/journal.rs（Stage 3.1）。

#[test]
fn non_failure_final_status_preserves_clarify_semantics() {
    assert_eq!(
        non_failure_final_status(false),
        crate::task_journal::TaskJournalFinalStatus::Success
    );
    assert_eq!(
        non_failure_final_status(true),
        crate::task_journal::TaskJournalFinalStatus::Clarify
    );
}

#[test]
fn answer_verifier_high_confidence_gap_forces_task_failure() {
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["command_output".to_string()],
        answer_incomplete_reason: "answer omitted requested synthesis".to_string(),
        should_retry: true,
        retry_instruction: "use the observed command output".to_string(),
        confidence: 0.85,
    });

    assert!(answer_verifier_forces_task_failure(false, &journal));
    assert!(!answer_verifier_forces_task_failure(true, &journal));
    let payload: serde_json::Value = serde_json::from_str(
        &journal
            .answer_verifier_summary
            .as_ref()
            .expect("summary")
            .required_evidence_failure_payload_text(),
    )
    .expect("structured payload");
    assert_eq!(
        payload
            .get("message_key")
            .and_then(serde_json::Value::as_str),
        Some("answer_verifier_required_evidence_block")
    );
    assert_eq!(
        payload
            .pointer("/status_code")
            .and_then(serde_json::Value::as_str),
        Some("answer_verifier_required_evidence_block")
    );
    assert_eq!(
        payload
            .pointer("/failure_attribution")
            .and_then(serde_json::Value::as_str),
        Some("answer_verifier_gap")
    );
    assert_eq!(
        payload
            .pointer("/retryable")
            .and_then(serde_json::Value::as_bool),
        Some(false)
    );
    assert_eq!(
        payload
            .pointer("/missing_evidence_fields/0")
            .and_then(serde_json::Value::as_str),
        Some("command_output")
    );
    assert!(!answer_verifier_should_force_task_failure(
        false, false, &journal
    ));
    assert!(answer_verifier_should_force_task_failure(
        true, false, &journal
    ));
    assert!(!answer_verifier_should_force_task_failure(
        true, true, &journal
    ));
    record_answer_verifier_required_evidence_rollout_attribution(&mut journal);
    assert_eq!(
        journal
            .to_summary_json()
            .pointer("/rollout_attribution/0/switch_name")
            .and_then(serde_json::Value::as_str),
        Some("answer_verifier_enforce_required_scope")
    );
    assert_eq!(
        journal
            .to_summary_json()
            .pointer("/rollout_attribution/0/reason_code")
            .and_then(serde_json::Value::as_str),
        Some("answer_verifier_required_evidence_block")
    );
}

#[test]
fn pure_chat_agent_loop_verifier_gap_triggers_direct_retry() {
    let mut route = route_result(crate::AskMode::act_with_chat_finalizer());
    route.route_reason = "pure_chat_agent_loop_submode".to_string();
    route.output_contract.requires_content_evidence = false;
    route.output_contract.delivery_required = false;
    route.wants_file_delivery = false;

    let journal =
        crate::task_journal::TaskJournal::for_task("task-1", "ask", "direct response request");
    let verifier = crate::answer_verifier::AnswerVerifierOut {
        pass: false,
        missing_evidence_fields: vec!["output_format".to_string()],
        answer_incomplete_reason: "candidate does not satisfy the requested output".to_string(),
        should_retry: true,
        retry_instruction: "rewrite from the original request".to_string(),
        confidence: 0.9,
    };

    assert!(answer_verifier_retry_applicable(
        &route, &journal, &verifier,
    ));
}

#[test]
fn observed_tool_evidence_verifier_gap_triggers_direct_retry() {
    let mut route = route_result(crate::AskMode::act_plain());
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.wants_file_delivery = false;

    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-1", "ask", "summarize observed evidence");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "archive_basic",
            r#"{"entries":[{"name":"notes.txt"}],"content":{"notes.txt":"alpha"}}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "db_basic",
            r#"{"tables":["app_meta"],"schema_version":3}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_3",
            "synthesize_answer",
            "partial answer",
        ));
    let verifier = crate::answer_verifier::AnswerVerifierOut {
        pass: false,
        missing_evidence_fields: vec!["field_value".to_string(), "output_format".to_string()],
        answer_incomplete_reason: "candidate omitted observed values".to_string(),
        should_retry: true,
        retry_instruction: "rewrite from observed step outputs".to_string(),
        confidence: 0.93,
    };

    assert!(answer_verifier_retry_applicable(
        &route, &journal, &verifier,
    ));
    let observed_trace = answer_verifier_retry_observed_trace(&journal);
    assert!(observed_trace.contains("archive_basic"));
    assert!(observed_trace.contains("db_basic"));
    assert!(observed_trace.contains("schema_version"));
}

#[test]
fn observed_tool_evidence_retry_ignores_failed_tool_step() {
    let mut route = route_result(crate::AskMode::act_plain());
    route.output_contract.requires_content_evidence = true;

    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-1", "ask", "summarize observed evidence");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::new(
            "step_1",
            "archive_basic",
            crate::executor::StepExecutionStatus::Error,
            None,
            Some(r#"{"error_kind":"permission_denied"}"#.to_string()),
        ));
    let verifier = crate::answer_verifier::AnswerVerifierOut {
        pass: false,
        missing_evidence_fields: vec!["field_value".to_string()],
        answer_incomplete_reason: "candidate omitted observed values".to_string(),
        should_retry: true,
        retry_instruction: "rewrite from observed step outputs".to_string(),
        confidence: 0.93,
    };

    assert!(!answer_verifier_retry_applicable(
        &route, &journal, &verifier,
    ));
}

#[test]
fn requested_machine_kv_summary_final_guard_replaces_raw_observation_answer() {
    let prompt = "Return exactly machine summary command=python3 scripts/sync_skill_docs.py";
    let route = route_result(crate::AskMode::act_plain());
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-machine-kv-final", "ask", prompt);
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            r#"{"excerpt":"144| - `python3 scripts/sync_skill_docs.py`"}"#,
        ));
    let mut answer_text = "144| - `python3 scripts/sync_skill_docs.py`".to_string();
    let mut answer_messages = vec![answer_text.clone()];

    assert!(apply_requested_machine_kv_summary_to_final_answer(
        prompt,
        &route,
        &mut journal,
        &mut answer_text,
        &mut answer_messages,
    ));

    assert_eq!(answer_text, "command=python3 scripts/sync_skill_docs.py");
    assert_eq!(
        answer_messages,
        vec!["command=python3 scripts/sync_skill_docs.py".to_string()]
    );
    assert_eq!(
        journal.final_answer.as_deref(),
        Some("command=python3 scripts/sync_skill_docs.py")
    );
}

#[test]
fn requested_machine_kv_summary_final_guard_preserves_colon_field_values() {
    let prompt = "Return text_excerpt and detected_format.";
    let mut route = route_result(crate::AskMode::act_plain());
    route.resolved_intent = "text_excerpt detected_format".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptWithSummary;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-machine-kv-colon-fields", "ask", prompt);
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            r#"{"extra":{"action":"read_range","excerpt":"1|Archive fixtures for NL tests.","path":"/tmp/README.txt"}}"#,
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "synthesize_answer",
            "text_excerpt: \"Archive fixtures for NL tests.\"\ndetected_format: plain text",
        ));
    let mut answer_text =
        "text_excerpt: \"Archive fixtures for NL tests.\"\ndetected_format: plain text".to_string();
    let mut answer_messages = vec![answer_text.clone()];

    assert!(!apply_requested_machine_kv_summary_to_final_answer(
        prompt,
        &route,
        &mut journal,
        &mut answer_text,
        &mut answer_messages,
    ));

    assert_eq!(
        answer_text,
        "text_excerpt: \"Archive fixtures for NL tests.\"\ndetected_format: plain text"
    );
    assert_eq!(answer_messages, vec![answer_text.clone()]);
    assert_eq!(journal.final_answer.as_deref(), Some(answer_text.as_str()));
}

#[test]
fn requested_machine_kv_summary_failure_recovery_projects_read_range_fields() {
    let prompt = "用 read_range 读取 docs/service_notes.md 第 1 到 6 行，最终只回答机器字段 path 和 total_lines。";
    let mut route = route_result(crate::AskMode::act_plain());
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route
        .output_contract
        .self_extension
        .structured_field_selector = Some("total_lines".to_string());
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-read-range-machine-kv", "ask", prompt);
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec![
            "output_format".to_string(),
            "path".to_string(),
            "total_lines".to_string(),
        ],
        answer_incomplete_reason: "candidate omitted requested machine fields".to_string(),
        should_retry: true,
        retry_instruction: "use observed read_range machine fields".to_string(),
        confidence: 0.95,
    });
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            r#"{"extra":{"action":"read_range","path":"/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/docs/service_notes.md","resolved_path":"/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/docs/service_notes.md","start_line":1,"end_line":7,"total_lines":7,"excerpt":"1|# Service Notes\n2|."}}"#,
        ));
    let mut answer_text = "# Service Notes\n\nRustClaw test fixture service notes.".to_string();
    let mut answer_messages = vec![answer_text.clone()];

    assert!(recover_requested_machine_kv_summary_final_answer(
        prompt,
        &route,
        &mut journal,
        &mut answer_text,
        &mut answer_messages,
        true,
    ));

    assert_eq!(
        answer_text,
        "path=/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/docs/service_notes.md total_lines=7"
    );
    assert_eq!(answer_messages, vec![answer_text.clone()]);
    assert!(journal
        .answer_verifier_summary
        .as_ref()
        .is_some_and(|summary| summary.pass));
    assert_eq!(journal.final_answer.as_deref(), Some(answer_text.as_str()));
}

#[test]
fn requested_machine_kv_summary_final_guard_preserves_compare_paths_existence_fields() {
    let prompt = "return same_path=false and both exist fields";
    let mut route = route_result(crate::AskMode::act_plain());
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RecentScalarEqualityCheck;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-compare-paths-final", "ask", prompt);
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            r#"{"extra":{"action":"compare_paths","field_value":{"same_path":false,"left_exists":true,"right_exists":true}}}"#,
        ));
    let mut answer_text = "same_path=false\nleft_exists=true\nright_exists=true".to_string();
    let mut answer_messages = vec![answer_text.clone()];

    assert!(!apply_requested_machine_kv_summary_to_final_answer(
        prompt,
        &route,
        &mut journal,
        &mut answer_text,
        &mut answer_messages,
    ));

    assert_eq!(
        answer_text,
        "same_path=false\nleft_exists=true\nright_exists=true"
    );
    assert_eq!(answer_messages, vec![answer_text.clone()]);
    assert_eq!(journal.final_answer.as_deref(), Some(answer_text.as_str()));
}

#[test]
fn requested_machine_kv_summary_final_guard_preserves_content_evidence_synthesis() {
    let prompt = "Read README.md first 20 lines and answer existence, line count, and title.";
    let mut route = route_result(crate::AskMode::act_plain());
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_hint = "README.md".to_string();
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-content-evidence-final", "ask", prompt);
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            r#"{"extra":{"path":"/home/guagua/rustclaw/README.md","resolved_path":"/home/guagua/rustclaw/README.md","excerpt":"1|# RustClaw\n2|body","end_line":20}}"#,
        ));
    let mut answer_text =
        "文件存在；成功读取前 20 行；标题（第一行 `# RustClaw`）中出现 RustClaw。".to_string();
    let mut answer_messages = vec![answer_text.clone()];

    assert!(!apply_requested_machine_kv_summary_to_final_answer(
        prompt,
        &route,
        &mut journal,
        &mut answer_text,
        &mut answer_messages,
    ));

    assert!(answer_text.contains("RustClaw"));
    assert_eq!(answer_messages, vec![answer_text.clone()]);
    assert!(journal.final_answer.as_deref() != Some("README.md"));
}

#[test]
fn requested_machine_kv_summary_final_guard_preserves_generated_file_report_fields() {
    let prompt = "return dry_run=true provider/model planned_outputs and output_path";
    let mut route = route_result(crate::AskMode::act_plain());
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::GeneratedFilePathReport;
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-generated-file-report-final",
        "ask",
        prompt,
    );
    let mut answer_text = concat!(
        "dry_run=true\n",
        "provider=minimax\n",
        "model=image-01\n",
        "output_path=/home/guagua/rustclaw/document/media_dry_run/image_status_card.png\n",
        "planned_outputs=[{\"path\":\"/home/guagua/rustclaw/document/media_dry_run/image_status_card.png\",\"type\":\"image_file\"}]"
    )
    .to_string();
    let mut answer_messages = vec![answer_text.clone()];

    assert!(!apply_requested_machine_kv_summary_to_final_answer(
        prompt,
        &route,
        &mut journal,
        &mut answer_text,
        &mut answer_messages,
    ));

    assert!(answer_text.contains("output_path="));
    assert!(answer_text.contains("planned_outputs="));
    assert_eq!(answer_messages, vec![answer_text.clone()]);
}

#[test]
fn requested_machine_kv_summary_final_guard_preserves_async_poll_report_fields() {
    let prompt = "return task_id job_id status and async_poll_adapter_result";
    let mut route = route_result(crate::AskMode::act_plain());
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::GeneratedFilePathReport;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-async-poll-report-final", "ask", prompt);
    let mut answer_text = concat!(
        "task_id: image-task-001\n",
        "job_id: image-job-001\n",
        "status: succeeded\n",
        "async_poll_adapter_result: {\"adapter_kind\":\"media_job_poll\",\"status\":\"succeeded\"}"
    )
    .to_string();
    let mut answer_messages = vec![answer_text.clone()];

    assert!(!apply_requested_machine_kv_summary_to_final_answer(
        prompt,
        &route,
        &mut journal,
        &mut answer_text,
        &mut answer_messages,
    ));

    assert!(answer_text.contains("task_id: image-task-001"));
    assert!(answer_text.contains("async_poll_adapter_result:"));
    assert_eq!(answer_messages, vec![answer_text.clone()]);
}

#[test]
fn requested_machine_kv_summary_final_guard_ignores_internal_route_tokens() {
    let prompt = "Return async timeout policy fields.";
    let mut route = route_result(crate::AskMode::act_plain());
    route.route_reason = "current_workspace_scope_from_current_request=false".to_string();
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-machine-kv-internal", "ask", prompt);
    journal.record_context_bundle_summary(
        "current_workspace_scope_from_current_request=false".to_string(),
    );
    journal.record_route_result(&route);
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1", "respond", "false",
        ));
    let mut answer_text = "false".to_string();
    let mut answer_messages = vec![answer_text.clone()];

    assert!(!apply_requested_machine_kv_summary_to_final_answer(
        prompt,
        &route,
        &mut journal,
        &mut answer_text,
        &mut answer_messages,
    ));
    assert_eq!(answer_text, "false");
    assert_eq!(answer_messages, vec!["false".to_string()]);
}

#[test]
fn existing_file_delivery_token_answer_canonicalizes_workspace_relative_path() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "rustclaw_finalize_file_token_{}_{}",
        std::process::id(),
        nonce
    ));
    let logs = root.join("logs");
    std::fs::create_dir_all(&logs).expect("create logs dir");
    let file = logs.join("clawd-codex-current.log");
    std::fs::write(&file, "ok\n").expect("write file");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.clone();

    let normalized =
        normalize_existing_file_delivery_token_answer(&state, "FILE:logs/clawd-codex-current.log")
            .expect("normalize file token");

    assert_eq!(
        normalized,
        format!("FILE:{}", file.canonicalize().unwrap().display())
    );
    assert!(
        normalize_existing_file_delivery_token_answer(&state, "FILE:logs/missing.log").is_none()
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn delivery_path_gap_without_observation_finalizes_as_clarify() {
    let mut route = route_result(crate::AskMode::act_plain());
    route.output_contract.delivery_required = true;
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::GeneratedFileDelivery;

    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-delivery-clarify", "ask", "prompt");
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["path".to_string()],
        answer_incomplete_reason: "missing_required_evidence:path".to_string(),
        should_retry: false,
        retry_instruction: "collect_required_evidence_fields:path".to_string(),
        confidence: 0.0,
    });
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "respond".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some("needs_input".to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    });
    journal.record_final_stop_signal("respond");

    assert!(delivery_path_gap_should_finalize_as_clarify(
        &route,
        "needs_input",
        &["needs_input".to_string()],
        &journal,
    ));
    assert!(!delivery_path_gap_should_finalize_as_clarify(
        &route,
        "FILE:/tmp/result.txt",
        &[],
        &journal,
    ));

    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(r#"{"path":"/tmp/result.txt"}"#.to_string()),
        error: None,
        started_at: 3,
        finished_at: 4,
    });
    assert!(!delivery_path_gap_should_finalize_as_clarify(
        &route,
        "needs_input",
        &[],
        &journal,
    ));
}

#[test]
fn ask_runtime_failure_payload_is_machine_readable() {
    let payload: serde_json::Value = serde_json::from_str(&ask_runtime_failure_machine_payload(
        r#"provider=vendor-minimax failed: http 429: {"error":{"type":"rate_limit_error"}}"#,
    ))
    .unwrap();
    assert_eq!(
        payload
            .pointer("/message_key")
            .and_then(serde_json::Value::as_str),
        Some("clawd.msg.ask_runtime_failure")
    );
    assert_eq!(
        payload
            .pointer("/reason_code")
            .and_then(serde_json::Value::as_str),
        Some("ask_runtime_failure")
    );
    assert_eq!(
        payload
            .pointer("/status_code")
            .and_then(serde_json::Value::as_str),
        Some("ask_runtime_failure")
    );
    assert_eq!(
        payload
            .pointer("/failure_attribution")
            .and_then(serde_json::Value::as_str),
        Some("provider_gap")
    );
    assert_eq!(
        payload
            .pointer("/retryable")
            .and_then(serde_json::Value::as_bool),
        Some(false)
    );
    assert_eq!(
        payload
            .pointer("/raw_error_present")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        payload
            .pointer("/provider_error_class")
            .and_then(serde_json::Value::as_str),
        Some("rate_limited")
    );
    assert!(payload.pointer("/error_summary").is_none());
}

#[test]
fn ask_runtime_failure_observed_facts_use_machine_payload_fields() {
    let facts = machine_payload_observed_facts(&ask_runtime_failure_machine_payload(
        r#"provider=vendor-minimax failed: http 429: {"error":{"type":"rate_limit_error"}}"#,
    ));
    assert!(facts.contains(&"message_key: clawd.msg.ask_runtime_failure".to_string()));
    assert!(facts.contains(&"status_code: ask_runtime_failure".to_string()));
    assert!(facts.contains(&"failure_attribution: provider_gap".to_string()));
    assert!(facts.contains(&"retryable: false".to_string()));
    assert!(facts.contains(&"raw_error_present: true".to_string()));
    assert!(facts.contains(&"provider_error_class: rate_limited".to_string()));
    assert!(!facts.iter().any(|fact| fact.starts_with("error_summary:")));
}

#[test]
fn failed_task_lifecycle_payload_marks_provider_gap_terminal_reason() {
    let payload = failed_task_lifecycle_payload(
        r#"provider=vendor-minimax failed: http 429: {"error":{"type":"rate_limit_error"}}"#,
    );

    assert_eq!(payload["state"], "failed");
    assert_eq!(payload["source"], "ask_failure_finalize");
    assert_eq!(payload["can_poll"], true);
    assert_eq!(payload["can_cancel"], false);
    assert_eq!(payload["failure_attribution"], "provider_error");
    assert_eq!(payload["terminal_reason"], "provider_window_exhausted");
}

#[test]
fn failed_task_lifecycle_payload_marks_structured_provider_unavailable_reason() {
    let err = crate::skills::structured_skill_error_from_parts(
        "llm",
        "provider_unavailable",
        "provider_unavailable",
        None,
        Some(json!({
            "error_code": "provider_unavailable",
            "message_key": "provider.unavailable"
        })),
    );
    let payload = failed_task_lifecycle_payload(&err);

    assert_eq!(payload["state"], "failed");
    assert_eq!(payload["failure_attribution"], "provider_error");
    assert_eq!(payload["terminal_reason"], "provider_window_exhausted");
}

#[test]
fn failed_task_lifecycle_payload_marks_confirmation_timeout_reason() {
    let err = crate::skills::structured_skill_error_from_parts(
        "policy",
        "confirmation_timeout",
        "confirmation_timeout",
        None,
        Some(json!({
            "error_code": "confirmation_timeout",
            "message_key": "policy.confirmation_timeout"
        })),
    );
    let payload = failed_task_lifecycle_payload(&err);

    assert_eq!(payload["state"], "failed");
    assert_eq!(payload["terminal_reason"], "confirmation_timeout");
}

#[test]
fn failed_task_lifecycle_payload_marks_verifier_terminal_reason() {
    let payload = failed_task_lifecycle_payload(
        r#"answer_verifier_required_evidence_block missing_required_evidence"#,
    );

    assert_eq!(payload["state"], "failed");
    assert_eq!(payload["failure_attribution"], "contract_gap");
    assert_eq!(payload["terminal_reason"], "verifier_unrecoverable");
}

#[test]
fn failed_task_lifecycle_payload_marks_tool_timeout_terminal_reason() {
    let err = crate::skills::structured_skill_error_from_parts(
        "run_cmd",
        "timeout",
        "timeout",
        None,
        Some(json!({
            "error_code": "timeout",
            "message_key": "clawd.run_cmd.timeout"
        })),
    );
    let payload = failed_task_lifecycle_payload(&err);

    assert_eq!(payload["state"], "failed");
    assert_eq!(
        payload["terminal_reason"],
        "tool_timeout_without_async_resume"
    );
}

#[test]
fn checkpointed_nonterminal_lifecycle_requires_matching_checkpoint() {
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-checkpointed", "ask", "prompt");
    journal.record_task_lifecycle(json!({
        "state": "waiting",
        "checkpoint_id": "ckpt-1",
        "resume_reason": "budget_near_exhaustion"
    }));
    journal.record_task_checkpoint(json!({
        "checkpoint_id": "ckpt-1",
        "resume_entrypoint": "next_planner_round"
    }));

    assert!(journal_has_checkpointed_nonterminal_lifecycle(&journal));

    journal.record_task_lifecycle(json!({
        "state": "succeeded",
        "checkpoint_id": "ckpt-1"
    }));
    assert!(!journal_has_checkpointed_nonterminal_lifecycle(&journal));

    journal.record_task_lifecycle(json!({
        "state": "waiting",
        "checkpoint_id": "ckpt-other"
    }));
    assert!(!journal_has_checkpointed_nonterminal_lifecycle(&journal));
}

#[test]
fn answer_verifier_failure_machine_json_is_detected() {
    assert!(answer_text_is_machine_json_payload(
        r#"{"message_key":"answer_verifier_required_evidence_block","missing_evidence_fields":["output_format"]}"#,
    ));
    assert!(!answer_text_is_machine_json_payload("rustclaw"));
}

#[test]
fn answer_verifier_failure_fallback_line_is_not_json() {
    let line = answer_verifier_failure_machine_line(
        r#"{"message_key":"answer_verifier_required_evidence_block","missing_evidence_fields":["output_format"]}"#,
    );
    assert!(line.contains("message_key=answer_verifier_required_evidence_block"));
    assert!(line.contains("missing_evidence_fields=output_format"));
    assert!(serde_json::from_str::<serde_json::Value>(&line).is_err());
}

#[test]
fn answer_verifier_failure_err_json_triggers_user_message_path() {
    assert!(super::answer_verifier_failure_needs_user_message(
        "Host ThinkPad-X1 is observable.",
        r#"{"message_key":"answer_verifier_required_evidence_block","answer_incomplete_reason":"shape"}"#,
    ));
}

#[test]
fn answer_verifier_failure_returns_machine_payload_without_fallback_llm() {
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.policy.schedule.locale = "en-US".to_string();
    let task = crate::ClaimedTask {
        task_id: "task-verifier-i18n".to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };

    let visible = compose_answer_verifier_failure_user_message(
        &state,
        &task,
        "please answer in English",
        r#"{"message_key":"answer_verifier_required_evidence_block","missing_evidence_fields":["output_format"]}"#,
    );

    let payload: serde_json::Value =
        serde_json::from_str(&visible).expect("verifier failure should stay machine-readable");
    assert_eq!(
        payload
            .get("message_key")
            .and_then(serde_json::Value::as_str),
        Some("answer_verifier_required_evidence_block")
    );
    assert_eq!(
        payload
            .get("missing_evidence_fields")
            .and_then(serde_json::Value::as_array)
            .and_then(|items| items.first())
            .and_then(serde_json::Value::as_str),
        Some("output_format")
    );
}

#[test]
fn answer_verifier_failure_machine_line_triggers_user_message_path() {
    assert!(super::answer_verifier_failure_needs_user_message(
        "message_key=answer_verifier_required_evidence_block missing_evidence_fields=content_excerpt",
        "",
    ));
}

#[test]
fn answer_verifier_failure_observed_facts_use_machine_fields() {
    let facts = answer_verifier_failure_observed_facts(
        r#"{"message_key":"answer_verifier_required_evidence_block","reason_code":"answer_verifier_required_evidence_block","status_code":"answer_verifier_required_evidence_block","failure_attribution":"answer_verifier_gap","retryable":false,"missing_evidence_fields":["output_format"],"answer_incomplete_reason":"shape"}"#,
    );
    assert!(facts.contains(&"message_key: answer_verifier_required_evidence_block".to_string()));
    assert!(facts.contains(&"status_code: answer_verifier_required_evidence_block".to_string()));
    assert!(facts.contains(&"failure_attribution: answer_verifier_gap".to_string()));
    assert!(facts.contains(&"retryable: false".to_string()));
    assert!(facts.contains(&"missing_evidence_fields: output_format".to_string()));
    assert!(facts.contains(&"answer_incomplete_reason: shape".to_string()));
}

#[test]
fn answer_verifier_failure_default_payload_preserves_machine_fields() {
    let payload = answer_verifier_failure_default_payload(
        "message_key=answer_verifier_required_evidence_block missing_evidence_fields=content_excerpt,output_format",
    );
    let value: serde_json::Value = serde_json::from_str(&payload).expect("machine payload");
    assert_eq!(
        value.get("message_key").and_then(serde_json::Value::as_str),
        Some("answer_verifier_required_evidence_block")
    );
    assert_eq!(
        value
            .pointer("/missing_evidence_fields/0")
            .and_then(serde_json::Value::as_str),
        Some("content_excerpt")
    );
    assert_eq!(
        answer_verifier_failure_missing_fields_text(&payload),
        "content_excerpt,output_format"
    );
}

#[test]
fn answer_verifier_failure_machine_line_observed_facts_add_defaults() {
    let facts = answer_verifier_failure_observed_facts(
        "message_key=answer_verifier_required_evidence_block missing_evidence_fields=content_excerpt",
    );
    assert!(facts.contains(&"message_key: answer_verifier_required_evidence_block".to_string()));
    assert!(facts.contains(&"reason_code: answer_verifier_required_evidence_block".to_string()));
    assert!(facts.contains(&"status_code: answer_verifier_required_evidence_block".to_string()));
    assert!(facts.contains(&"failure_attribution: answer_verifier_gap".to_string()));
    assert!(facts.contains(&"retryable: false".to_string()));
    assert!(facts.contains(&"missing_evidence_fields: content_excerpt".to_string()));
}

#[test]
fn filtered_log_entry_gap_recovers_from_read_range_observation() {
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-filtered-log", "ask", "prompt");
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["filtered_entry".to_string()],
        answer_incomplete_reason: "filtered entry missing".to_string(),
        should_retry: true,
        retry_instruction: "filter observed log entry".to_string(),
        confidence: 0.85,
    });
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "extra": {
                    "action": "read_range",
                    "mode": "tail",
                    "requested_n": 4,
                    "path": "logs/clawd.run.log",
                    "resolved_path": "/workspace/logs/clawd.run.log",
                    "excerpt": "8|2026-05-27T08:04:44Z INFO task_call\n9|2026-05-27T08:04:45Z WARN removed_think_block\n10|2026-05-27T08:04:46Z ERROR provider timeout"
                },
                "text": "{}"
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let recovered =
        deterministic_filtered_log_entry_recovery(&journal).expect("filtered log entry recovery");

    assert!(recovered.contains("log.filtered_entry.level=ERROR"));
    assert!(recovered.contains("log.filtered_entry.line=10"));
    assert!(recovered.contains("provider timeout"));
    assert!(!recovered.contains("removed_think_block"));
}

#[test]
fn raw_tail_read_failure_recovery_returns_observed_excerpt() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let task = crate::ClaimedTask {
        task_id: "task-raw-tail-recovery".to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let mut route = route_result(crate::AskMode::act_plain());
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/workspace/logs/clawd-dev.log".to_string();
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-raw-tail-recovery", "ask", "prompt");
    journal.record_finalizer_summary(crate::task_journal::TaskJournalFinalizerSummary {
        stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
        disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
        contract_ok: true,
        used_evidence_ids_count: 1,
        ..Default::default()
    });
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "extra": {
                    "action": "read_range",
                    "mode": "tail",
                    "requested_n": 2,
                    "path": "/workspace/logs/clawd-dev.log",
                    "resolved_path": "/workspace/logs/clawd-dev.log",
                    "excerpt": "98|WARN provider failed: http 401: credential_missing\n99|WARN memory preference fallback failed: http 401"
                },
                "text": "{}"
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let recovered = deterministic_raw_tail_read_failure_recovery(
        &state,
        &task,
        "read tail lines",
        &route,
        &journal,
    )
    .expect("raw tail read should recover from observed evidence");

    assert_eq!(
        recovered,
        "WARN provider failed: http 401: credential_missing\nWARN memory preference fallback failed: http 401"
    );
}

#[test]
fn tree_summary_rows_failure_recovery_returns_machine_fields() {
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-tree-summary-recovery", "ask", "prompt");
    journal.record_final_stop_signal("synthesize_answer_failed");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "system_basic",
            json!({
                "action": "tree_summary",
                "summary_rows": [
                    {
                        "path": "scripts/nl_tests/fixtures/device_local",
                        "name": "device_local",
                        "kind": "dir",
                        "file_count": 2,
                        "truncated": false
                    },
                    {
                        "path": "scripts/nl_tests/fixtures/device_local/docs",
                        "name": "docs",
                        "kind": "dir",
                        "file_count": 2,
                        "truncated": false
                    }
                ]
            })
            .to_string(),
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "system_basic",
            json!({
                "action": "tree_summary",
                "summary_rows": [
                    {
                        "path": "scripts/nl_tests/fixtures/device_local/_unpacked_notes",
                        "name": "_unpacked_notes",
                        "kind": "dir",
                        "file_count": 1,
                        "truncated": false
                    }
                ]
            })
            .to_string(),
        ));

    let recovered = deterministic_tree_summary_rows_failure_recovery(&journal)
        .expect("tree summary rows recovery");

    assert_eq!(
        recovered,
        "name=device_local file_count=2 truncated=false\nname=docs file_count=2 truncated=false"
    );
}

#[test]
fn content_tail_read_failure_recovery_selects_observed_log_line() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let task = crate::ClaimedTask {
        task_id: "task-content-tail-recovery".to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let mut route = route_result(crate::AskMode::act_plain());
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/workspace/logs/clawd.run.log".to_string();
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-content-tail-recovery", "ask", "prompt");
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["content_excerpt".to_string()],
        answer_incomplete_reason: "candidate omitted observed values".to_string(),
        should_retry: true,
        retry_instruction: "rewrite from observed step outputs".to_string(),
        confidence: 0.9,
    });
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "extra": {
                    "action": "read_range",
                    "mode": "tail",
                    "requested_n": 3,
                    "path": "/workspace/logs/clawd.run.log",
                    "resolved_path": "/workspace/logs/clawd.run.log",
                    "excerpt": "10|2026-06-25T09:12:01Z INFO task_call: executor_step_start step=1\n11|2026-06-25T09:12:02Z INFO task_call: verifier_result approved=true issue_count=0\n12|2026-06-25T09:12:03Z WARN task_call: answer_verifier_observed_gap missing_evidence=content_excerpt"
                },
                "text": "{}"
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let recovered = deterministic_content_tail_read_failure_recovery(
        &state,
        &task,
        "read tail lines",
        &route,
        &journal,
    )
    .expect("content tail read should recover from observed evidence");

    assert_eq!(
        recovered,
        "2026-06-25T09:12:03Z WARN task_call: answer_verifier_observed_gap missing_evidence=content_excerpt"
    );
}

#[test]
fn config_guard_candidates_recovery_uses_nested_observed_evidence() {
    let mut route = route_result(crate::AskMode::act_plain());
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ConfigRiskAssessment;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-config-guard-candidates-recovery",
        "ask",
        "prompt",
    );
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["candidates".to_string()],
        answer_incomplete_reason: "candidate evidence missing".to_string(),
        should_retry: true,
        retry_instruction: "use guard candidates".to_string(),
        confidence: 0.9,
    });
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "config_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "extra": {
                    "action": "guard_config",
                    "path": "configs/config.toml",
                    "risk_count": 2,
                    "candidates": [
                        "tools.allow_sudo=true",
                        "tools.allow_path_outside_workspace=true"
                    ],
                    "valid": false
                },
                "text": "{}"
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let recovered =
        deterministic_config_guard_candidates_recovery(&route, &journal).expect("recovery");
    let payload: serde_json::Value = serde_json::from_str(&recovered).expect("json payload");

    assert_eq!(
        payload
            .get("reason_code")
            .and_then(serde_json::Value::as_str),
        Some("config_edit_guard_risk_found")
    );
    assert_eq!(
        payload.get("count").and_then(serde_json::Value::as_u64),
        Some(2)
    );
    assert_eq!(
        payload
            .pointer("/candidates/1")
            .and_then(serde_json::Value::as_str),
        Some("tools.allow_path_outside_workspace=true")
    );
    assert_eq!(
        payload.get("valid").and_then(serde_json::Value::as_bool),
        Some(false)
    );
}

#[test]
fn config_guard_candidates_recovery_handles_truncated_output_excerpt() {
    let mut route = route_result(crate::AskMode::act_plain());
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ConfigRiskAssessment;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-config-guard-truncated-recovery",
        "ask",
        "prompt",
    );
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["candidates".to_string()],
        answer_incomplete_reason: "candidate evidence missing".to_string(),
        should_retry: true,
        retry_instruction: "use guard candidates".to_string(),
        confidence: 0.9,
    });
    journal.step_results.push(crate::task_journal::TaskJournalStepTrace::ok(
        "step_1",
        "config_basic",
        r#"{"extra":{"action":"guard_config","candidates":["tools.allow_sudo=true","tools.allow_path_outside_workspace=true"],"count":2,"format":"toml","path":"configs/config.toml","resolved_path":"/workspace/configs/config.toml","risk_coun...(truncated)"#,
    ));

    let recovered =
        deterministic_config_guard_candidates_recovery(&route, &journal).expect("recovery");
    let payload: serde_json::Value = serde_json::from_str(&recovered).expect("json payload");

    assert_eq!(
        payload
            .get("risk_count")
            .and_then(serde_json::Value::as_u64),
        Some(2)
    );
    assert_eq!(
        payload
            .pointer("/candidates/0")
            .and_then(serde_json::Value::as_str),
        Some("tools.allow_sudo=true")
    );
    assert_eq!(
        payload.get("path").and_then(serde_json::Value::as_str),
        Some("configs/config.toml")
    );
}

#[test]
fn config_guard_candidates_recovery_allows_validation_route_after_guard_observation() {
    let mut route = route_result(crate::AskMode::act_plain());
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ConfigValidation;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-config-validation-guard-recovery",
        "ask",
        "prompt",
    );
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["candidates".to_string(), "field_value".to_string()],
        answer_incomplete_reason: "candidate evidence missing".to_string(),
        should_retry: true,
        retry_instruction: "use guard candidates".to_string(),
        confidence: 0.95,
    });
    journal.step_results.push(crate::task_journal::TaskJournalStepTrace::ok(
        "step_1",
        "config_basic",
        r#"{"extra":{"action":"guard_config","candidates":["tools.allow_sudo=true"],"count":1,"path":"configs/config.toml"}}"#,
    ));

    let recovered =
        deterministic_config_guard_candidates_recovery(&route, &journal).expect("recovery");
    let payload: serde_json::Value = serde_json::from_str(&recovered).expect("json payload");

    assert_eq!(
        payload
            .pointer("/candidates/0")
            .and_then(serde_json::Value::as_str),
        Some("tools.allow_sudo=true")
    );
    assert_eq!(
        payload.get("count").and_then(serde_json::Value::as_u64),
        Some(1)
    );
}

#[test]
fn assistant_memory_source_text_filters_execution_summary() {
    let messages = vec![
        "**执行过程**\n1. 调用命令 `pwd`\n   输出：\n```text\n/tmp\n```".to_string(),
        "最终答案".to_string(),
    ];

    assert_eq!(
        assistant_memory_source_text("最终答案", &messages),
        "最终答案"
    );
}

#[test]
fn assistant_memory_source_text_filters_machine_execution_summary() {
    let messages = vec![
        json!({
            "message_key": "clawd.msg.execution.summary",
            "reason_code": "resume_failed_step_summary",
            "action": "skill(run_cmd)",
            "error": "Command failed with exit code 127"
        })
        .to_string(),
        "final answer".to_string(),
    ];

    assert_eq!(
        assistant_memory_source_text("final answer", &messages),
        "final answer"
    );
}

#[test]
fn assistant_memory_source_text_drops_execution_summary_only_answers() {
    let messages = vec![
        "**执行过程**\n1. 调用技能 `rss_fetch`\n   输出：ok".to_string(),
        "**执行过程**\n1. 调用技能 `rss_fetch`\n   输出：ok".to_string(),
    ];

    assert_eq!(
        assistant_memory_source_text(
            "**执行过程**\n1. 调用技能 `rss_fetch`\n   输出：ok",
            &messages
        ),
        ""
    );
}

#[test]
fn scalar_delivery_does_not_reinsert_execution_summary() {
    let mut route = route_result(crate::AskMode::Act {
        finalize: crate::ActFinalizeStyle::Plain,
    });
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;

    assert!(!should_reinsert_execution_summaries_for_delivery(
        &route, "1.0.0"
    ));
}

#[test]
fn scalar_delivery_drops_existing_execution_summary_messages() {
    let mut route = route_result(crate::AskMode::Act {
        finalize: crate::ActFinalizeStyle::Plain,
    });
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    let mut messages = vec![
        "**执行过程**\n1. 调用工具 `fs_basic`\n   输出：ok".to_string(),
        "{\"workspace\":true}".to_string(),
    ];

    drop_execution_summaries_when_delivery_is_scalar(&route, "{\"workspace\":true}", &mut messages);

    assert_eq!(messages, vec!["{\"workspace\":true}".to_string()]);
}

#[test]
fn strict_structured_delivery_drops_existing_execution_summary_messages() {
    let mut route = route_result(crate::AskMode::Act {
        finalize: crate::ActFinalizeStyle::Plain,
    });
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    let answer = r#"{"info":17,"warn":2,"error":1}"#;
    let mut messages = vec![
        "**执行过程**\n1. 调用技能 `log_analyze`\n   输出：ok".to_string(),
        answer.to_string(),
    ];

    assert!(!should_reinsert_execution_summaries_for_delivery(
        &route, answer
    ));
    drop_execution_summaries_when_delivery_is_scalar(&route, answer, &mut messages);

    assert_eq!(messages, vec![answer.to_string()]);
}

#[test]
fn strict_file_delivery_keeps_execution_summary_available() {
    let mut route = route_result(crate::AskMode::Act {
        finalize: crate::ActFinalizeStyle::Plain,
    });
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.delivery_required = true;

    assert!(should_reinsert_execution_summaries_for_delivery(
        &route,
        r#"{"file":"report.md"}"#
    ));
}

#[test]
fn config_validation_delivery_drops_existing_execution_summary_messages() {
    let mut route = route_result(crate::AskMode::Act {
        finalize: crate::ActFinalizeStyle::ChatWrapped,
    });
    route.route_reason = "capability_ref=config.validate".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    let mut messages = vec![
        "**Execution**\n1. Called tool `config_basic`\n   Output: valid".to_string(),
        "pass".to_string(),
    ];

    drop_execution_summaries_when_delivery_is_scalar(&route, "pass", &mut messages);

    assert_eq!(messages, vec!["pass".to_string()]);
}

#[test]
fn free_delivery_keeps_execution_summary_available() {
    let mut route = route_result(crate::AskMode::Act {
        finalize: crate::ActFinalizeStyle::Plain,
    });
    route.output_contract.response_shape = crate::OutputResponseShape::Free;

    assert!(should_reinsert_execution_summaries_for_delivery(
        &route,
        "配置检查通过。"
    ));
}

#[test]
fn journal_missing_file_search_evidence_detects_zero_match_fs_search() {
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            skill: "fs_search".to_string(),
            output_excerpt: Some(
                json!({
                    "action": "find_name",
                    "count": 0,
                    "results": [],
                    "root": ""
                })
                .to_string(),
            ),
            ..Default::default()
        });
    assert!(journal_has_missing_file_search_evidence(Some(&journal)));
}

#[test]
fn journal_missing_file_search_evidence_detects_path_batch_facts() {
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            skill: "system_basic".to_string(),
            output_excerpt: Some(
                json!({
                    "action": "path_batch_facts",
                    "count": 1,
                    "facts": [{
                        "exists": false,
                        "path": "/tmp/missing.txt",
                        "error": "not found"
                    }],
                    "include_missing": true
                })
                .to_string(),
            ),
            ..Default::default()
        });
    assert!(journal_has_missing_file_search_evidence(Some(&journal)));
}

#[test]
fn journal_missing_file_search_evidence_detects_not_found_probe() {
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            skill: "run_cmd".to_string(),
            output_excerpt: Some("NOT_FOUND\n".to_string()),
            ..Default::default()
        });
    assert!(journal_has_missing_file_search_evidence(Some(&journal)));
}

#[test]
fn answer_route_result_overrides_initial_chat_when_execution_trace_exists() {
    let initial = route_result(crate::AskMode::respond_trace());
    let answer_route = route_result(crate::AskMode::act_with_chat_finalizer());
    let mut answer_journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    answer_journal.record_plan_result(&crate::PlanResult {
        plan_kind: crate::PlanKind::Single,
        goal: "inspect project".to_string(),
        planner_notes: String::new(),
        raw_plan_text: String::new(),
        missing_slots: Vec::new(),
        needs_confirmation: false,
        steps: Vec::new(),
    });

    assert!(should_use_answer_route_result(
        &initial,
        &answer_route,
        &answer_journal
    ));
}

#[test]
fn answer_route_result_does_not_override_chat_without_execution_trace() {
    let initial = route_result(crate::AskMode::respond_trace());
    let answer_route = route_result(crate::AskMode::act_with_chat_finalizer());
    let answer_journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");

    assert!(!should_use_answer_route_result(
        &initial,
        &answer_route,
        &answer_journal
    ));
}

#[test]
fn answer_route_result_overrides_initial_chat_for_clarify_journal() {
    let initial = route_result(crate::AskMode::respond_trace());
    let mut answer_route = route_result(crate::AskMode::clarify_trace());
    answer_route.needs_clarify = true;
    answer_route.clarify_question = "Which file should I send?".to_string();
    answer_route.wants_file_delivery = true;
    answer_route.output_contract.delivery_required = true;
    answer_route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    let mut answer_journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    answer_journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Clarify);

    assert!(should_use_answer_route_result(
        &initial,
        &answer_route,
        &answer_journal
    ));
}

#[test]
fn journal_missing_file_search_evidence_detects_read_file_error_marker() {
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            skill: "read_file".to_string(),
            error_excerpt: Some("__RC_READ_FILE_NOT_FOUND__:/tmp/missing.txt".to_string()),
            ..Default::default()
        });
    assert!(journal_has_missing_file_search_evidence(Some(&journal)));
}

#[test]
fn missing_file_delivery_reply_uses_structured_search_evidence() {
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            skill: "fs_search".to_string(),
            output_excerpt: Some(
                json!({
                    "action": "find_name",
                    "count": 0,
                    "results": [],
                    "root": ""
                })
                .to_string(),
            ),
            ..Default::default()
        });
    let answer = crate::AskReply::llm(
        "文件 `definitely_missing_named_file_rustclaw_001.txt` 未找到。".to_string(),
    )
    .with_task_journal(journal);
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "send definitely_missing_named_file_rustclaw_001.txt".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "explicit filename".to_string(),
        route_confidence: Some(1.0),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: true,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract::default(),
    };
    assert!(route.wants_file_delivery);
    assert!(journal_has_missing_file_search_evidence(
        answer.task_journal.as_ref()
    ));
}

#[test]
fn missing_file_delivery_reply_uses_output_contract_file_token_even_without_wants_flag() {
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            skill: "fs_search".to_string(),
            output_excerpt: Some(
                json!({
                    "action": "find_name",
                    "count": 0,
                    "results": [],
                    "root": ""
                })
                .to_string(),
            ),
            ..Default::default()
        });
    let answer = crate::AskReply::llm(
        "找不到文件 `definitely_missing_named_file_rustclaw_001.txt`。".to_string(),
    )
    .with_task_journal(journal);
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
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
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.output_contract.delivery_required = true;

    assert!(super::should_use_missing_file_delivery_reply(
        &route, &answer
    ));
}

#[test]
fn resume_failure_missing_file_delivery_is_success_result() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
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
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.output_contract.delivery_required = true;
    let resume_ctx = json!({
        "failed_step": {
            "action": "skill(run_cmd)",
            "error": "__RC_READ_FILE_NOT_FOUND__:/tmp/missing.txt"
        },
        "remaining_actions": []
    });

    assert!(super::resume_failure_is_missing_file_delivery_result(
        &route,
        "I couldn't send the requested file because it doesn't exist at the path `/tmp/missing.txt`.",
        &resume_ctx
    ));
}

#[test]
fn resume_failure_unbound_path_lookup_is_clarify_result() {
    let mut route = route_result(crate::AskMode::act_plain());
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "case_only/report.md".to_string();
    let resume_ctx = json!({
        "completed_messages": [
            "subtask#1 skill(system_basic): success\n{\"action\":\"path_batch_facts\",\"count\":1,\"facts\":[{\"error\":\"not found\",\"exists\":false,\"kind\":\"missing\",\"path\":\"case_only/report.md\"}],\"include_missing\":true}"
        ],
        "failed_step": {
            "action": "skill(fs_search)",
            "error": "read_dir failed",
            "structured_error": {
                "skill": "fs_search",
                "error_kind": "unknown",
                "error_text": "read_dir failed"
            }
        },
        "remaining_actions": []
    });

    assert!(resume_context_path_batch_facts_are_missing_only(
        &resume_ctx
    ));
    assert!(resume_failure_is_unbound_path_lookup_clarify_result(
        &route,
        &resume_ctx
    ));
}

#[test]
fn resume_failure_unbound_directory_lookup_is_clarify_result_without_path_batch() {
    let mut route = route_result(crate::AskMode::act_plain());
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = "case_only/report.md".to_string();
    let resume_ctx = json!({
        "completed_messages": [],
        "failed_step": {
            "action": "skill(fs_search)",
            "error": "read_dir failed: No such file or directory (os error 2)",
            "structured_error": {
                "skill": "fs_search",
                "error_kind": "unknown",
                "error_text": "read_dir failed: No such file or directory (os error 2)"
            }
        },
        "remaining_actions": []
    });

    assert!(resume_context_has_directory_lookup_failure(&resume_ctx));
    assert!(resume_failure_is_unbound_path_lookup_clarify_result(
        &route,
        &resume_ctx
    ));
}

#[test]
fn resume_failure_unbound_path_lookup_does_not_reclassify_delivery() {
    let mut route = route_result(crate::AskMode::act_plain());
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.output_contract.delivery_required = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
    let resume_ctx = json!({
        "completed_messages": [
            "subtask#1 skill(system_basic): success\n{\"action\":\"path_batch_facts\",\"count\":1,\"facts\":[{\"exists\":false,\"path\":\"missing.txt\"}],\"include_missing\":true}"
        ],
        "failed_step": {
            "action": "skill(fs_search)",
            "error": "read_dir failed"
        },
        "remaining_actions": []
    });

    assert!(!resume_failure_is_unbound_path_lookup_clarify_result(
        &route,
        &resume_ctx
    ));
}

#[test]
fn resume_failure_structured_service_status_is_success_result() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
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
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ServiceStatus;
    let resume_ctx = json!({
        "failed_step": {
            "action": "skill(service_control)",
            "error": "no matching service found for the given target",
            "structured_error": {
                "skill": "service_control",
                "error_kind": "not_found",
                "error_text": "no matching service found for the given target",
                "service_name": "definitely_missing_rustclaw_demo",
                "platform": "linux",
                "manager_type": "unknown"
            }
        },
        "remaining_actions": []
    });

    assert!(super::resume_failure_is_structured_service_status_result(
        &route,
        &resume_ctx
    ));

    let messages = super::resume_context_execution_summary_messages(&resume_ctx, false);
    assert_eq!(messages.len(), 1);
    let summary: serde_json::Value = serde_json::from_str(&messages[0]).unwrap();
    assert_eq!(
        summary
            .pointer("/message_key")
            .and_then(serde_json::Value::as_str),
        Some("clawd.msg.execution.summary")
    );
    assert_eq!(
        summary
            .pointer("/reason_code")
            .and_then(serde_json::Value::as_str),
        Some("resume_failed_step_summary")
    );
    assert!(summary
        .pointer("/error")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|error| error.contains("no matching service found")));
    assert!(!messages[0].contains("__RC_SKILL_ERROR__"));
}

#[test]
fn resume_failure_execution_failed_step_is_success_answer_with_remaining_actions() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
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
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExecutionFailedStep;
    let resume_ctx = json!({
        "failed_step": {
            "action": "skill(run_cmd)",
            "error": "command failed with exit code 1; stderr: cat: /definitely_missing_rustclaw_contract_case: No such file or directory (os error 2)",
            "structured_error": {
                "skill": "run_cmd",
                "error_kind": "nonzero_exit",
                "error_text": "Command failed with exit code 1\nstderr:\ncat: /definitely_missing_rustclaw_contract_case: No such file or directory (os error 2)",
                "platform": "linux",
                "extra": {
                    "command": "cat /definitely_missing_rustclaw_contract_case",
                    "exit_code": 1,
                    "stderr": "cat: /definitely_missing_rustclaw_contract_case: No such file or directory (os error 2)\n"
                }
            }
        },
        "remaining_actions": [
            {"type": "call_skill", "skill": "log_analyze"},
            {"type": "synthesize_answer"}
        ]
    });

    let answer = super::resume_failure_execution_failed_step_answer(&route, &resume_ctx, false)
        .expect("execution-failed-step answer");

    let payload: serde_json::Value = serde_json::from_str(&answer).unwrap();
    assert_eq!(
        payload
            .pointer("/message_key")
            .and_then(serde_json::Value::as_str),
        Some("clawd.msg.execution.failed_step")
    );
    assert_eq!(
        payload
            .pointer("/reason_code")
            .and_then(serde_json::Value::as_str),
        Some("execution_failed_step")
    );
    assert_eq!(
        payload
            .pointer("/command")
            .and_then(serde_json::Value::as_str),
        Some("cat /definitely_missing_rustclaw_contract_case")
    );
    assert_eq!(
        payload
            .pointer("/exit_code")
            .and_then(serde_json::Value::as_i64),
        Some(1)
    );
    assert!(payload
        .pointer("/detail")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|detail| detail.contains("No such file or directory")));
}

#[test]
fn resume_context_execution_summary_uses_failed_step() {
    let resume_ctx = json!({
        "failed_step": {
            "action": "skill(run_cmd)",
            "error": "ls: cannot access '/tmp/missing.txt': No such file or directory"
        },
        "remaining_actions": []
    });

    let messages = super::resume_context_execution_summary_messages(&resume_ctx, false);

    assert_eq!(messages.len(), 1);
    assert!(crate::finalize::is_execution_summary_message(&messages[0]));
    let summary: serde_json::Value = serde_json::from_str(&messages[0]).unwrap();
    assert_eq!(
        summary
            .pointer("/message_key")
            .and_then(serde_json::Value::as_str),
        Some("clawd.msg.execution.summary")
    );
    assert_eq!(
        summary
            .pointer("/action")
            .and_then(serde_json::Value::as_str),
        Some("skill(run_cmd)")
    );
    assert!(summary
        .pointer("/error")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|error| error.contains("No such file or directory")));
}

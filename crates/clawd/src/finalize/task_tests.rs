use super::{
    answer_text_is_machine_json_payload, answer_verifier_failure_default_payload,
    answer_verifier_failure_machine_line, answer_verifier_failure_missing_fields_text,
    answer_verifier_failure_observed_facts, answer_verifier_forces_task_failure,
    answer_verifier_retry_applicable, answer_verifier_retry_observed_trace,
    answer_verifier_should_force_task_failure, apply_verified_terminal_answer_after_verifier_pass,
    ask_runtime_failure_default_text, ask_runtime_failure_machine_payload,
    assistant_memory_source_text, bounded_answer_retry_prompt,
    compose_answer_verifier_failure_user_message, delivery_path_gap_should_finalize_as_clarify,
    drop_execution_summaries_when_delivery_is_scalar, failed_task_lifecycle_payload,
    finalize_ask_checkpointed, finalize_ask_result, journal_has_checkpointed_nonterminal_lifecycle,
    journal_has_missing_file_search_evidence, machine_payload_observed_facts,
    non_failure_final_status, normalize_existing_file_delivery_token_answer,
    planner_output_contract_for_finalization,
    record_answer_verifier_required_evidence_rollout_attribution,
    resume_context_has_directory_lookup_failure, resume_context_path_batch_facts_are_missing_only,
    resume_failure_is_unbound_path_lookup_clarify_result,
    should_reinsert_execution_summaries_for_delivery,
};

use serde_json::json;

#[path = "task_tests/answer_verifier_recovery.rs"]
mod answer_verifier_recovery;
#[path = "task_tests/checkpoint_finalization.rs"]
mod checkpoint_finalization;
#[path = "task_tests/content_evidence_delivery.rs"]
mod content_evidence_delivery;
#[path = "task_tests/final_status.rs"]
mod final_status;
#[path = "task_tests/runtime_failure_payload.rs"]
mod runtime_failure_payload;
#[path = "task_tests/verified_terminal_promotion.rs"]
mod verified_terminal_promotion;

fn route_result() -> crate::IntentOutputContract {
    crate::IntentOutputContract::default()
}

#[test]
fn finalization_uses_planner_contract_from_answer_journal() {
    let mut route = route_result();
    route.response_shape = crate::OutputResponseShape::FileToken;
    route.delivery_required = true;
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
    journal.record_output_contract(&route.clone());

    let selected = planner_output_contract_for_finalization(Some(&journal));

    assert_eq!(
        selected.response_shape,
        crate::OutputResponseShape::FileToken
    );
    assert!(selected.delivery_required);
}

#[test]
fn finalization_uses_machine_fallback_when_planner_contract_is_unavailable() {
    let selected = planner_output_contract_for_finalization(None);

    assert!(selected.does_not_request_exact_command_output());
    assert!(!selected.delivery_required);
}

// ensure_journal_task_metrics_* tests 已搬移到 finalize/journal.rs（Stage 3.1）。

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
fn direct_answer_verifier_gap_triggers_direct_retry() {
    let mut route = route_result();
    route.requires_content_evidence = false;
    route.delivery_required = false;

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
    let mut route = route_result();
    route.requires_content_evidence = true;
    route.delivery_required = false;

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
fn bounded_answer_retry_uses_structured_issue_without_verifier_prose() {
    const REASON_SENTINEL: &str = "reason-prose-must-not-enter-retry-prompt";
    const INSTRUCTION_SENTINEL: &str = "instruction-prose-must-not-enter-retry-prompt";
    let route = route_result();
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "request");
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["content_excerpt".to_string()],
        answer_incomplete_reason: REASON_SENTINEL.to_string(),
        should_retry: true,
        retry_instruction: INSTRUCTION_SENTINEL.to_string(),
        confidence: 0.91,
    });
    let verifier = crate::answer_verifier::AnswerVerifierOut {
        pass: false,
        missing_evidence_fields: vec!["content_excerpt".to_string()],
        answer_incomplete_reason: REASON_SENTINEL.to_string(),
        should_retry: true,
        retry_instruction: INSTRUCTION_SENTINEL.to_string(),
        confidence: 0.91,
    };

    let observed_trace = answer_verifier_retry_observed_trace(&journal);
    let template =
        include_str!("../../../../prompts/layers/overlays/answer_verifier_retry_prompt.md");
    let prompt = bounded_answer_retry_prompt(
        template,
        "en",
        "same_as_user",
        "request",
        &route,
        "context",
        &observed_trace,
        "rejected answer",
        &verifier,
    );

    assert!(prompt.contains("\"missing_evidence_fields\":[\"content_excerpt\"]"));
    assert!(prompt.contains("\"should_retry\":true"));
    assert!(!observed_trace.contains(REASON_SENTINEL));
    assert!(!observed_trace.contains(INSTRUCTION_SENTINEL));
    assert!(!prompt.contains(REASON_SENTINEL));
    assert!(!prompt.contains(INSTRUCTION_SENTINEL));
}

#[test]
fn observed_tool_evidence_retry_ignores_failed_tool_step() {
    let mut route = route_result();
    route.requires_content_evidence = true;

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
    let mut route = route_result();
    route.delivery_required = true;
    route.response_shape = crate::OutputResponseShape::FileToken;
    route.delivery_intent = crate::OutputDeliveryIntent::FileSingle;

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
        "resume_reason": "task_budget_slice_exhausted"
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
        claim_attempt: 0,
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
    let mut route = route_result();
    route.response_shape = crate::OutputResponseShape::Scalar;

    assert!(!should_reinsert_execution_summaries_for_delivery(
        &route, "1.0.0"
    ));
}

#[test]
fn scalar_delivery_drops_existing_execution_summary_messages() {
    let mut route = route_result();
    route.response_shape = crate::OutputResponseShape::Scalar;
    let mut messages = vec![
        "**执行过程**\n1. 调用工具 `fs_basic`\n   输出：ok".to_string(),
        "{\"workspace\":true}".to_string(),
    ];

    drop_execution_summaries_when_delivery_is_scalar(&route, "{\"workspace\":true}", &mut messages);

    assert_eq!(messages, vec!["{\"workspace\":true}".to_string()]);
}

#[test]
fn strict_structured_delivery_drops_existing_execution_summary_messages() {
    let mut route = route_result();
    route.response_shape = crate::OutputResponseShape::Strict;
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
    let mut route = route_result();
    route.response_shape = crate::OutputResponseShape::Strict;
    route.delivery_required = true;

    assert!(should_reinsert_execution_summaries_for_delivery(
        &route,
        r#"{"file":"report.md"}"#
    ));
}

#[test]
fn free_delivery_keeps_execution_summary_available() {
    let mut route = route_result();
    route.response_shape = crate::OutputResponseShape::Free;

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
fn missing_file_search_evidence_is_detected_without_route_hint() {
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
    let mut route = crate::IntentOutputContract::default();
    route.response_shape = crate::OutputResponseShape::FileToken;
    route.delivery_required = true;

    assert!(super::should_use_missing_file_delivery_reply(
        &route, &answer
    ));
}

#[test]
fn resume_failure_missing_file_delivery_is_success_result() {
    let mut route = crate::IntentOutputContract::default();
    route.response_shape = crate::OutputResponseShape::FileToken;
    route.delivery_required = true;
    let resume_ctx = json!({
        "failed_step": {
            "action": "skill(run_cmd)",
            "error": "__RC_READ_FILE_NOT_FOUND__:/tmp/missing.txt",
            "structured_error": {
                "skill": "read_file",
                "error_kind": "not_found",
                "extra": {
                    "path": "/tmp/missing.txt",
                    "error_code": "not_found"
                }
            }
        },
        "remaining_actions": []
    });

    assert!(super::resume_failure_is_missing_file_delivery_result(
        &route,
        &resume_ctx
    ));
}

#[test]
fn resume_failure_missing_file_delivery_rejects_prose_only_error() {
    let mut route = crate::IntentOutputContract::default();
    route.response_shape = crate::OutputResponseShape::FileToken;
    route.delivery_required = true;
    let resume_ctx = json!({
        "failed_step": {
            "action": "skill(run_cmd)",
            "error": "I couldn't send the requested file because it doesn't exist."
        },
        "remaining_actions": []
    });

    assert!(!super::resume_failure_is_missing_file_delivery_result(
        &route,
        &resume_ctx
    ));
}

#[test]
fn resume_failure_unbound_path_lookup_is_clarify_result() {
    let mut route = route_result();
    route.requires_content_evidence = true;
    route.response_shape = crate::OutputResponseShape::Scalar;
    route.selection.structured_field_selector = Some("path".to_string());
    route.locator_kind = crate::OutputLocatorKind::Path;
    route.locator_hint = "case_only/report.md".to_string();
    let resume_ctx = json!({
        "completed_messages": [
            "subtask#1 skill(system_basic): success\n{\"action\":\"path_batch_facts\",\"count\":1,\"facts\":[{\"error\":\"not found\",\"exists\":false,\"kind\":\"missing\",\"path\":\"case_only/report.md\"}],\"include_missing\":true}"
        ],
        "failed_step": {
            "action": "skill(fs_search)",
            "error": "read_dir failed",
            "structured_error": {
                "skill": "fs_search",
                "error_kind": "read_dir_failed",
                "extra": {
                    "operation": "read_dir",
                    "reason_code": "read_dir_failed"
                }
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
    let mut route = route_result();
    route.requires_content_evidence = true;
    route.response_shape = crate::OutputResponseShape::Scalar;
    route.selection.structured_field_selector = Some("path".to_string());
    route.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.locator_hint = "case_only/report.md".to_string();
    let resume_ctx = json!({
        "completed_messages": [],
        "failed_step": {
            "action": "skill(fs_search)",
            "error": "read_dir failed: No such file or directory (os error 2)",
            "structured_error": {
                "skill": "fs_search",
                "error_kind": "directory_lookup_failed",
                "extra": {
                    "operation": "read_dir",
                    "error_code": "directory_lookup_failed"
                }
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
    let mut route = route_result();
    route.requires_content_evidence = true;
    route.response_shape = crate::OutputResponseShape::FileToken;
    route.delivery_required = true;
    route.selection.structured_field_selector = Some("path".to_string());
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

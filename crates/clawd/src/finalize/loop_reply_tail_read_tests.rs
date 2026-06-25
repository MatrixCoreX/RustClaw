use super::*;

use crate::finalize::loop_reply::enforce_delivery_output_contract;

#[test]
fn tail_read_range_observed_answer_replaces_failed_synthesis_for_content_excerpt() {
    let state = test_state();
    let task = claimed_task("task-tail");
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "logs/clawd_manual.log".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state
        .delivery_messages
        .push("**执行过程**\n1. 调用技能 `system_basic`（action=read_range）".to_string());
    loop_state
        .delivery_messages
        .push("由于日志输出被截断，无法查看最后2行内容。".to_string());
    loop_state.last_user_visible_respond =
        Some("由于日志输出被截断，无法查看最后2行内容。".to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "system_basic",
        r#"{"action":"read_range","mode":"head","requested_n":40,"excerpt":"1|startup\n2|ready"}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "synthesize_answer",
        "由于日志输出被截断，无法查看最后2行内容。",
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_3",
        "system_basic",
        r#"{"action":"read_range","mode":"tail","requested_n":2,"excerpt":"4318|last alpha\n4319|last beta"}"#,
    ));
    let mut finalizer_summary = None;

    assert!(replace_delivery_with_latest_tail_read_range_answer(
        &state,
        &task,
        "看最后一个最后 2 行",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
    ));

    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("last alpha\nlast beta")
    );
    assert!(loop_state
        .delivery_messages
        .iter()
        .any(|message| crate::finalize::is_execution_summary_message(message)));
    assert_eq!(
        loop_state.delivery_messages.last().map(String::as_str),
        Some("last alpha\nlast beta")
    );
    assert_eq!(
        finalizer_summary.and_then(|summary| summary.disposition),
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn one_sentence_content_excerpt_tail_read_selects_observed_log_line() {
    let state = test_state();
    let task = claimed_task("task-tail-one-sentence");
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "logs/clawd.run.log".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state
        .delivery_messages
        .push("unsupported synthesis".to_string());
    loop_state.last_user_visible_respond = Some("unsupported synthesis".to_string());
    loop_state.last_publishable_synthesis_output = Some("unsupported synthesis".to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","mode":"tail","requested_n":4,"excerpt":"10|2026-06-25T09:10:01Z INFO task_call: executor_step_start step=1\n11|2026-06-25T09:10:02Z INFO task_call: task_journal_summary goal=### MEMORY_USE_POLICY\n12|2026-06-25T09:10:03Z WARN task_call: answer_verifier_observed_gap missing_evidence=unsupported_claims\n13|2026-06-25T09:10:04Z INFO task_call: verifier_result approved=true issue_count=0"}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "synthesize_answer",
        "unsupported synthesis",
    ));
    let mut finalizer_summary = None;

    assert!(replace_delivery_with_latest_tail_read_range_answer(
        &state,
        &task,
        "挑最值得注意的一行",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
    ));

    let answer = loop_state
        .last_user_visible_respond
        .as_deref()
        .unwrap_or("");
    assert_eq!(
        answer,
        "2026-06-25T09:10:03Z WARN task_call: answer_verifier_observed_gap missing_evidence=unsupported_claims"
    );
    assert!(!answer.contains("MEMORY_USE_POLICY"));
    assert!(!answer.contains("unsupported synthesis"));
    assert_eq!(
        loop_state.delivery_messages.last().map(String::as_str),
        Some(answer)
    );
    assert_eq!(
        finalizer_summary.and_then(|summary| summary.disposition),
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn one_sentence_excerpt_kind_tail_read_selects_observed_log_line() {
    let state = test_state();
    let task = claimed_task("task-tail-excerpt-kind");
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExcerptKindJudgment;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "logs/clawd.run.log".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state
        .delivery_messages
        .push("unsupported synthesis".to_string());
    loop_state.last_user_visible_respond = Some("unsupported synthesis".to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","mode":"tail","requested_n":3,"excerpt":"20|2026-06-25T09:11:01Z INFO task_call: executor_step_start step=1\n21|2026-06-25T09:11:02Z INFO task_call: verifier_result approved=true issue_count=0\n22|2026-06-25T09:11:03Z WARN task_call: answer_verifier_observed_gap missing_evidence=stale_round"}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "synthesize_answer",
        "unsupported synthesis",
    ));
    let mut finalizer_summary = None;

    assert!(replace_delivery_with_latest_tail_read_range_answer(
        &state,
        &task,
        "select one line",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
    ));

    let answer = loop_state
        .last_user_visible_respond
        .as_deref()
        .unwrap_or("");
    assert_eq!(
        answer,
        "2026-06-25T09:11:03Z WARN task_call: answer_verifier_observed_gap missing_evidence=stale_round"
    );
    assert!(!answer.contains("unsupported synthesis"));
    assert_eq!(
        loop_state.delivery_messages.last().map(String::as_str),
        Some(answer)
    );
    assert_eq!(
        finalizer_summary.and_then(|summary| summary.disposition),
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn tail_read_range_observed_answer_allows_malformed_none_semantic_fs_basic() {
    let state = test_state();
    let task = claimed_task("task-tail-none");
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "logs/model_io.log".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .delivery_messages
        .push("已有执行结果，但我没能整理成可靠结论。".to_string());
    loop_state.last_user_visible_respond =
        Some("已有执行结果，但我没能整理成可靠结论。".to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","mode":"tail","requested_n":2,"excerpt":"1548|{\"task_id\":\"task-1\",\"omitted_fields\":[\"prompt\"]}\n1549|{\"task_id\":\"task-2\",\"omitted_fields\":[\"prompt\"]}"}"#,
    ));
    let mut finalizer_summary = None;

    assert!(replace_delivery_with_latest_tail_read_range_answer(
        &state,
        &task,
        "看看最后 2 行",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
    ));

    let answer = loop_state
        .last_user_visible_respond
        .as_deref()
        .unwrap_or("");
    assert!(answer.contains("task-1"));
    assert!(answer.contains("task-2"));
    assert!(!answer.contains("已有执行结果"));
}

#[test]
fn tail_read_range_backfill_reads_extra_wrapped_fs_basic_output() {
    let task = claimed_task("task-tail-backfill-extra");
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "logs/clawd-dev.log".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"read_range","mode":"tail","requested_n":1,"excerpt":"99|fresh wrapped line","path":"logs/clawd-dev.log"}}"#,
    ));

    backfill_delivery_from_last_outputs(&task, &mut loop_state, Some(&agent_run_context));

    assert_eq!(
        loop_state.delivery_messages.last().map(String::as_str),
        Some("fresh wrapped line")
    );
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("fresh wrapped line")
    );
}

#[test]
fn strict_raw_tail_read_replaces_synthesized_failure_from_log_contents() {
    let state = test_state();
    let task = claimed_task("task-tail-replace-log-failure-synthesis");
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "logs/clawd-dev.log".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let synthesis = "The log shows HTTP 401 and says the task cannot continue.".to_string();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.delivery_messages.push(synthesis.clone());
    loop_state.last_user_visible_respond = Some(synthesis.clone());
    loop_state.last_publishable_synthesis_output = Some(synthesis.clone());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"read_range","mode":"tail","requested_n":2,"excerpt":"98|WARN provider failed: http 401\n99|WARN memory fallback failed: http 401","path":"logs/clawd-dev.log"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "synthesize_answer",
        &synthesis,
    ));
    let mut finalizer_summary = None;

    assert!(replace_delivery_with_latest_tail_read_range_answer(
        &state,
        &task,
        "read the last two lines",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
    ));

    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("WARN provider failed: http 401\nWARN memory fallback failed: http 401")
    );
    assert_eq!(
        loop_state.delivery_messages.last().map(String::as_str),
        Some("WARN provider failed: http 401\nWARN memory fallback failed: http 401")
    );
    assert!(finalizer_summary.is_some());
}

#[tokio::test]
async fn finalize_loop_reply_strict_raw_tail_read_overrides_synthesized_failure_text() {
    let state = test_state();
    let task = claimed_task("task-tail-finalize-replace-log-failure-synthesis");
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "logs/clawd-dev.log".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let synthesis = "Reading failed because the observed log line contains HTTP 401.".to_string();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.delivery_messages.push(synthesis.clone());
    loop_state.last_user_visible_respond = Some(synthesis.clone());
    loop_state.last_publishable_synthesis_output = Some(synthesis.clone());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"read_range","mode":"tail","requested_n":2,"excerpt":"98|WARN provider failed: http 401\n99|WARN memory fallback failed: http 401","path":"logs/clawd-dev.log"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "synthesize_answer",
        &synthesis,
    ));

    let reply = finalize_loop_reply(
        &state,
        &task,
        "read the last two lines",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should return the observed tail lines");

    assert!(!reply.should_fail_task, "reply: {}", reply.text);
    assert_eq!(
        reply.text.trim(),
        "WARN provider failed: http 401\nWARN memory fallback failed: http 401"
    );
    assert_eq!(
        reply
            .task_journal
            .as_ref()
            .and_then(|journal| journal.final_status),
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
}

#[tokio::test]
async fn enforce_contract_keeps_strict_raw_tail_read_with_error_like_log_text() {
    let state = test_state();
    let task = claimed_task("task-tail-contract-keeps-error-like-log-text");
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "logs/clawd-dev.log".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let observed = "WARN provider failed: http 401: Please carry the API secret key\nWARN memory preference fallback failed: http 401";
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.delivery_messages.push(observed.to_string());
    loop_state.last_user_visible_respond = Some(observed.to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"read_range","mode":"tail","requested_n":2,"excerpt":"98|WARN provider failed: http 401: Please carry the API secret key\n99|WARN memory preference fallback failed: http 401","path":"logs/clawd-dev.log"}}"#,
    ));

    enforce_delivery_output_contract(
        &state,
        &task,
        "read the last two lines",
        &mut loop_state,
        Some(&agent_run_context),
    )
    .await;

    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(observed)
    );
    assert_eq!(loop_state.delivery_messages, vec![observed.to_string()]);
}

#[test]
fn tail_read_range_replaces_machine_evidence_projection() {
    let state = test_state();
    let task = claimed_task("task-tail-machine-projection");
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "logs/clawd.log".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let machine_projection = "path=/home/guagua/rustclaw/logs/clawd.log\ncontent_excerpt:\n1|old";
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .delivery_messages
        .push(machine_projection.to_string());
    loop_state.last_user_visible_respond = Some(machine_projection.to_string());
    loop_state.last_output = Some(machine_projection.to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","mode":"tail","requested_n":2,"excerpt":"10|fresh alpha\n11|fresh beta","path":"logs/clawd.log"}"#,
    ));
    let mut finalizer_summary = None;

    assert!(looks_like_structured_machine_output(machine_projection));
    assert!(replace_delivery_with_latest_tail_read_range_answer(
        &state,
        &task,
        "看最近 2 行",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
    ));

    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("fresh alpha\nfresh beta")
    );
    assert_eq!(
        loop_state.delivery_messages.last().map(String::as_str),
        Some("fresh alpha\nfresh beta")
    );
    assert!(finalizer_summary.is_some());
}

#[tokio::test]
async fn content_evidence_failure_defers_when_latest_tail_read_range_available() {
    let state = test_state();
    let task = claimed_task("task-tail-failure-defers");
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "logs/model_io.log".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(err_step_result(
        "step_1",
        "synthesize_answer",
        "synthesis failed",
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "fs_basic",
        r#"{"action":"read_range","mode":"tail","requested_n":2,"excerpt":"1|last alpha\n2|last beta"}"#,
    ));

    assert!(super::super::content_evidence_step_failure_reply_from_loop(
        &state,
        &task,
        "看看最后 2 行",
        &loop_state,
        Some(&agent_run_context),
    )
    .await
    .is_none());
}

#[test]
fn tail_read_range_observed_answer_selects_line_for_one_sentence_summary() {
    let state = test_state();
    let task = claimed_task("task-tail-summary");
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "system_basic",
        r#"{"action":"read_range","mode":"tail","requested_n":2,"excerpt":"1|a\n2|b"}"#,
    ));
    let mut finalizer_summary = None;

    assert!(replace_delivery_with_latest_tail_read_range_answer(
        &state,
        &task,
        "一句话总结最后两行",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
    ));
    assert_eq!(loop_state.last_user_visible_respond.as_deref(), Some("b"));
}

#[test]
fn tail_read_range_observed_answer_preserves_existing_content_summary() {
    let state = test_state();
    let task = claimed_task("task-tail-preserve-summary");
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "logs/clawd.run.log".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let summary = "最后几行都是同一任务的工具调度记录。".to_string();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .delivery_messages
        .push("**执行过程**\n1. 调用技能 `system_basic`（action=read_range）".to_string());
    loop_state.delivery_messages.push(summary.clone());
    loop_state.last_user_visible_respond = Some(summary.clone());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "system_basic",
        r#"{"action":"read_range","mode":"tail","requested_n":2,"excerpt":"1|raw alpha\n2|raw beta"}"#,
    ));
    let mut finalizer_summary = None;

    assert!(!replace_delivery_with_latest_tail_read_range_answer(
        &state,
        &task,
        "查看最后两行，只做简短概述",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
    ));
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(summary.as_str())
    );
    assert_eq!(
        loop_state.delivery_messages.last().map(String::as_str),
        Some(summary.as_str())
    );
    assert!(finalizer_summary.is_none());
}

#[test]
fn tail_read_range_observed_answer_replaces_older_summary_when_tail_synthesized_after_read() {
    let state = test_state();
    let task = claimed_task("task-tail-after-summary");
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "logs/model_io.log".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let older_summary = "model_io.log 里 error、failed、timeout 各出现 1 次。".to_string();
    let raw_tail_answer =
        "2026-05-20T09:00:00Z INFO prompt queued\n2026-05-20T09:00:01Z ERROR model timeout";
    let mut loop_state = crate::agent_engine::LoopState::new(4);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .delivery_messages
        .push("**执行过程**\n1. 调用技能 `log_analyze`（action=summarize）".to_string());
    loop_state.delivery_messages.push(older_summary.clone());
    loop_state.last_user_visible_respond = Some(older_summary);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "log_analyze",
        r#"{"action":"summarize","counts":{"error":1,"failed":1,"timeout":1}}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "synthesize_answer",
        "model_io.log 里 error、failed、timeout 各出现 1 次。",
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_3",
        "fs_basic",
        r#"{"action":"read_range","mode":"tail","requested_n":2,"excerpt":"31|2026-05-20T09:00:00Z INFO prompt queued\n32|2026-05-20T09:00:01Z ERROR model timeout"}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_4",
        "synthesize_answer",
        raw_tail_answer,
    ));
    let mut finalizer_summary = None;

    assert!(replace_delivery_with_latest_tail_read_range_answer(
        &state,
        &task,
        "看下最近 2 行",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
    ));
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(raw_tail_answer)
    );
    assert_eq!(
        loop_state.delivery_messages.last().map(String::as_str),
        Some(raw_tail_answer)
    );
    assert_eq!(
        finalizer_summary.and_then(|summary| summary.disposition),
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn tail_read_range_observed_answer_preserves_latest_registered_respond() {
    let state = test_state();
    let task = claimed_task("task-tail-preserve-respond");
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "logs/clawd.run.log".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let summary = "最后几行都是同一任务的工具调度记录。".to_string();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .delivery_messages
        .push("**执行过程**\n1. 调用技能 `system_basic`（action=read_range）".to_string());
    loop_state.delivery_messages.push(summary.clone());
    loop_state.last_user_visible_respond = Some(summary.clone());
    loop_state.last_output = Some(summary.clone());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "system_basic",
        r#"{"action":"read_range","mode":"tail","requested_n":2,"excerpt":"1|raw alpha\n2|raw beta"}"#,
    ));
    let mut finalizer_summary = None;

    assert!(!replace_delivery_with_latest_tail_read_range_answer(
        &state,
        &task,
        "查看最后两行，只做简短概述",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
    ));
    assert_eq!(
        loop_state.delivery_messages.last().map(String::as_str),
        Some(summary.as_str())
    );
    assert!(finalizer_summary.is_none());
}

#[test]
fn tail_read_range_observed_answer_replaces_synthesis_after_tail_for_strict_raw_output() {
    let state = test_state();
    let task = claimed_task("task-tail-preserve-synthesis-after-tail");
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "logs/model_io.log".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let raw_tail_json = r#"{"action":"read_range","mode":"tail","requested_n":5,"excerpt":"7|{\"status\":\"ok\",\"model\":\"gpt-4o-mini\",\"prompt_source\":\"clarify\"}\n8|{\"status\":\"ok\",\"model\":\"gpt-4o-mini\",\"prompt_source\":\"context\"}\n9|{\"status\":\"ok\",\"model\":\"gpt-4o-mini\",\"prompt_source\":\"context\"}"}"#;
    let synthesis = "{\"status\":\"ok\",\"model\":\"gpt-4o-mini\",\"prompt_source\":\"clarify\"}\n{\"status\":\"ok\",\"model\":\"gpt-4o-mini\",\"prompt_source\":\"context\"}\n{\"status\":\"ok\",\"model\":\"gpt-4o-mini\",\"prompt_source\":\"context\"}\n\nAll records are ok and show one continuous model-handled task flow."
        .to_string();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.delivery_messages.push(synthesis.clone());
    loop_state.last_user_visible_respond = Some(synthesis.clone());
    loop_state.last_publishable_synthesis_output = Some(synthesis.clone());
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "fs_basic", raw_tail_json));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "synthesize_answer",
        &synthesis,
    ));
    let mut finalizer_summary = None;

    assert!(replace_delivery_with_latest_tail_read_range_answer(
        &state,
        &task,
        "tail logs/model_io.log and provide the requested takeaway",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
    ));
    let raw_answer = "{\"status\":\"ok\",\"model\":\"gpt-4o-mini\",\"prompt_source\":\"clarify\"}\n{\"status\":\"ok\",\"model\":\"gpt-4o-mini\",\"prompt_source\":\"context\"}\n{\"status\":\"ok\",\"model\":\"gpt-4o-mini\",\"prompt_source\":\"context\"}";
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(raw_answer)
    );
    assert_eq!(
        loop_state.delivery_messages.last().map(String::as_str),
        Some(raw_answer)
    );
    assert_eq!(
        finalizer_summary.and_then(|summary| summary.disposition),
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

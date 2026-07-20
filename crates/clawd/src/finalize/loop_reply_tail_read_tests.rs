use super::*;

use crate::finalize::loop_reply::enforce_delivery_output_contract;

#[test]
fn generic_content_tail_read_does_not_replace_failed_synthesis() {
    let state = test_state();
    let task = claimed_task("task-tail");
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = OutputResponseShape::Strict;
    route.semantic_kind = crate::OutputSemanticKind::None;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "logs/clawd_manual.log".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
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

    assert!(!replace_delivery_with_latest_tail_read_range_answer(
        &state,
        &task,
        "看最后一个最后 2 行",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
    ));

    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("由于日志输出被截断，无法查看最后2行内容。")
    );
    assert!(finalizer_summary.is_none());
}

#[test]
fn tail_read_directory_inventory_projection_uses_planned_tail_count() {
    let state = test_state();
    let task = claimed_task("task-tail-directory-inventory");
    let route = free_route_result();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(4);
    loop_state.delivery_messages.push(
        "files.count=4\nfiles:\n- alpha.log\n- beta.log\n- gamma.log\n- omega.log".to_string(),
    );
    loop_state.last_user_visible_respond = loop_state.delivery_messages.last().cloned();
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            plan_result: Some(crate::PlanResult {
                goal: String::new(),
                missing_slots: Vec::new(),
                needs_confirmation: false,
                output_contract: None,
                steps: vec![crate::PlanStep {
                    step_id: "step_1".to_string(),
                    action_type: "call_tool".to_string(),
                    skill: "fs_basic".to_string(),
                    args: serde_json::json!({}),
                    depends_on: Vec::new(),
                    why: String::new(),
                }],
                planner_notes: String::new(),
                plan_kind: crate::PlanKind::Single,
                raw_plan_text: r#"{"steps":[{"type":"call_capability","capability":"filesystem.read_text_range","args":{"path":"/tmp/rustclaw/logs/base_skill_contracts","mode":"tail","n":2}}]}"#.to_string(),
            }),
            ..Default::default()
        });
    loop_state.executed_step_results.push(err_step_result(
        "step_1",
        "fs_basic",
        r#"__RC_SKILL_ERROR__:{"error_kind":"is_directory","error_text":"directory target","extra":null}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "fs_basic",
        r#"{"extra":{"action":"inventory_dir","path":"/tmp/rustclaw/logs/base_skill_contracts","resolved_path":"/tmp/rustclaw/logs/base_skill_contracts","names_by_kind":{"dirs":[],"files":["alpha.log","beta.log","gamma.log","omega.log"],"other":[]},"counts":{"dirs":0,"files":4,"hidden":0,"total":4},"sort_by":"name"}}"#,
    ));
    let mut finalizer_summary = Some(crate::task_journal::TaskJournalFinalizerSummary {
        stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
        disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
        contract_ok: true,
        completion_ok: Some(true),
        grounded_ok: Some(true),
        format_ok: Some(true),
        needs_clarify: Some(false),
        ..Default::default()
    });

    assert!(replace_delivery_with_latest_tail_read_range_answer(
        &state,
        &task,
        "tail selected directory",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
    ));

    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("entries.count=2\nentries:\n- gamma.log\n- omega.log")
    );
    assert_eq!(
        loop_state.delivery_messages,
        vec!["entries.count=2\nentries:\n- gamma.log\n- omega.log"]
    );
}

#[test]
fn bounded_head_read_range_observed_answer_replaces_failed_synthesis_for_content_excerpt() {
    let state = test_state();
    let task = claimed_task("task-head-read");
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = OutputResponseShape::Strict;
    route.semantic_kind = crate::OutputSemanticKind::None;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "scripts/nl_tests/fixtures/device_local/README.md".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    let stale_failure =
        "read_range completed, but final user-facing answer was not produced".to_string();
    loop_state.delivery_messages.push(stale_failure.clone());
    loop_state.last_user_visible_respond = Some(stale_failure);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"read_range","mode":"head","requested_n":10,"start_line":1,"end_line":9,"total_lines":9,"excerpt":"1|# Device Local Fixture\n2|\n3|This directory contains stable local files for RustClaw NL regression tests.\n4|\n5|- `configs/app_config.toml`: sample runtime config\n6|- `docs/`: sample docs and notes\n7|- `logs/`: sample log files\n8|- `data/test_contract.sqlite`: sample SQLite database\n9|- `tmp/test_bundle.zip`: sample archive","path":"scripts/nl_tests/fixtures/device_local/README.md"}}"#,
    ));
    loop_state.executed_step_results.push(err_step_result(
        "step_2",
        "synthesize_answer",
        "read_range completed, but final user-facing answer was not produced",
    ));
    let mut finalizer_summary = Some(crate::task_journal::TaskJournalFinalizerSummary {
        stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
        disposition: Some(crate::finalize::FinalizerDisposition::AllowFallback),
        parsed: true,
        contract_ok: false,
        completion_ok: Some(false),
        grounded_ok: Some(false),
        format_ok: Some(false),
        used_evidence_ids_count: 1,
        ..Default::default()
    });

    assert!(replace_delivery_with_latest_tail_read_range_answer(
        &state,
        &task,
        "read the first 10 lines",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
    ));

    let expected = "# Device Local Fixture\n\nThis directory contains stable local files for RustClaw NL regression tests.\n\n- `configs/app_config.toml`: sample runtime config\n- `docs/`: sample docs and notes\n- `logs/`: sample log files\n- `data/test_contract.sqlite`: sample SQLite database\n- `tmp/test_bundle.zip`: sample archive";
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(expected)
    );
    assert_eq!(loop_state.delivery_messages, vec![expected.to_string()]);
    assert_eq!(
        finalizer_summary.and_then(|summary| summary.disposition),
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn bounded_head_read_range_recovery_allows_unclassified_failed_free_route() {
    let state = test_state();
    let task = claimed_task("task-head-read-free-route");
    let mut route = free_route_result();
    route.response_shape = OutputResponseShape::Free;
    route.requires_content_evidence = false;
    route.semantic_kind = crate::OutputSemanticKind::None;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .delivery_messages
        .push("finalizer did not produce a publishable answer".to_string());
    loop_state.last_user_visible_respond =
        Some("finalizer did not produce a publishable answer".to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"read_range","mode":"head","requested_n":4,"start_line":1,"end_line":4,"total_lines":9,"excerpt":"1|# Device Local Fixture\n2|\n3|This directory contains stable local files for RustClaw NL regression tests.\n4|","path":"scripts/nl_tests/fixtures/device_local/README.md"}}"#,
    ));
    loop_state.executed_step_results.push(err_step_result(
        "step_2",
        "synthesize_answer",
        "synthesis failed",
    ));
    let mut finalizer_summary = Some(crate::task_journal::TaskJournalFinalizerSummary {
        stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
        disposition: Some(crate::finalize::FinalizerDisposition::AllowFallback),
        parsed: true,
        contract_ok: false,
        completion_ok: Some(false),
        grounded_ok: Some(false),
        format_ok: Some(false),
        used_evidence_ids_count: 1,
        ..Default::default()
    });

    assert!(replace_delivery_with_latest_tail_read_range_answer(
        &state,
        &task,
        "read the first 4 lines",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
    ));

    let expected = "# Device Local Fixture\n\nThis directory contains stable local files for RustClaw NL regression tests.";
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(expected)
    );
    assert_eq!(loop_state.delivery_messages, vec![expected.to_string()]);
    assert_eq!(
        finalizer_summary.and_then(|summary| summary.disposition),
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn generic_one_sentence_content_keeps_model_synthesis_authority() {
    let state = test_state();
    let task = claimed_task("task-tail-one-sentence");
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = OutputResponseShape::OneSentence;
    route.semantic_kind = crate::OutputSemanticKind::None;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "logs/clawd.run.log".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
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

    assert!(!replace_delivery_with_latest_tail_read_range_answer(
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
    assert_eq!(answer, "unsupported synthesis");
    assert_eq!(
        loop_state.delivery_messages.last().map(String::as_str),
        Some(answer)
    );
    assert!(finalizer_summary.is_none());
}

#[test]
fn tail_read_range_rejects_unclassified_content_contract() {
    let state = test_state();
    let task = claimed_task("task-tail-none");
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = OutputResponseShape::Strict;
    route.semantic_kind = crate::OutputSemanticKind::None;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "logs/model_io.log".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
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

    assert!(!replace_delivery_with_latest_tail_read_range_answer(
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
    assert_eq!(answer, "已有执行结果，但我没能整理成可靠结论。");
}

#[test]
fn tail_read_range_backfill_reads_extra_wrapped_fs_basic_output() {
    let task = claimed_task("task-tail-backfill-extra");
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = OutputResponseShape::Strict;
    route.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "logs/clawd-dev.log".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
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
fn tail_read_range_observed_answer_ignores_json_hidden_in_visible_text() {
    let state = test_state();
    let task = claimed_task("task-tail-text-boundary");
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = OutputResponseShape::Strict;
    route.semantic_kind = crate::OutputSemanticKind::None;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "logs/clawd-dev.log".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .delivery_messages
        .push("current synthesized summary".to_string());
    loop_state.last_user_visible_respond = Some("current synthesized summary".to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"text":"{\"action\":\"read_range\",\"mode\":\"tail\",\"requested_n\":1,\"excerpt\":\"99|hidden tail line\",\"path\":\"logs/clawd-dev.log\"}"}"#,
    ));
    let mut finalizer_summary = None;

    assert!(!replace_delivery_with_latest_tail_read_range_answer(
        &state,
        &task,
        "show the last log line",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
    ));

    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("current synthesized summary")
    );
    assert_eq!(
        loop_state.delivery_messages,
        vec!["current synthesized summary".to_string()]
    );
    assert!(finalizer_summary.is_none());
}

#[test]
fn strict_raw_tail_read_replaces_synthesized_failure_from_log_contents() {
    let state = test_state();
    let task = claimed_task("task-tail-replace-log-failure-synthesis");
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = OutputResponseShape::Strict;
    route.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "logs/clawd-dev.log".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
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
    route.requires_content_evidence = true;
    route.response_shape = OutputResponseShape::Strict;
    route.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "logs/clawd-dev.log".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
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
    route.requires_content_evidence = true;
    route.response_shape = OutputResponseShape::Strict;
    route.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "logs/clawd-dev.log".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
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
fn generic_content_tail_read_keeps_machine_projection_for_model_synthesis() {
    let state = test_state();
    let task = claimed_task("task-tail-machine-projection");
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = OutputResponseShape::Free;
    route.semantic_kind = crate::OutputSemanticKind::None;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "logs/clawd.log".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
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
    assert!(!replace_delivery_with_latest_tail_read_range_answer(
        &state,
        &task,
        "看最近 2 行",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
    ));

    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(machine_projection)
    );
    assert_eq!(
        loop_state.delivery_messages.last().map(String::as_str),
        Some(machine_projection)
    );
    assert!(finalizer_summary.is_none());
}

#[tokio::test]
async fn content_evidence_failure_defers_when_latest_tail_read_range_available() {
    let state = test_state();
    let task = claimed_task("task-tail-failure-defers");
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = OutputResponseShape::Strict;
    route.semantic_kind = crate::OutputSemanticKind::None;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "logs/model_io.log".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
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
fn generic_one_sentence_tail_read_does_not_select_a_line_in_runtime() {
    let state = test_state();
    let task = claimed_task("task-tail-summary");
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = OutputResponseShape::OneSentence;
    route.semantic_kind = crate::OutputSemanticKind::None;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "system_basic",
        r#"{"action":"read_range","mode":"tail","requested_n":2,"excerpt":"1|a\n2|b"}"#,
    ));
    let mut finalizer_summary = None;

    assert!(!replace_delivery_with_latest_tail_read_range_answer(
        &state,
        &task,
        "一句话总结最后两行",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
    ));
    assert!(loop_state.last_user_visible_respond.is_none());
}

#[test]
fn tail_read_range_observed_answer_preserves_existing_content_summary() {
    let state = test_state();
    let task = claimed_task("task-tail-preserve-summary");
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = OutputResponseShape::Strict;
    route.semantic_kind = crate::OutputSemanticKind::None;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "logs/clawd.run.log".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
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
fn generic_tail_read_does_not_replace_model_summary() {
    let state = test_state();
    let task = claimed_task("task-tail-after-summary");
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = OutputResponseShape::Strict;
    route.semantic_kind = crate::OutputSemanticKind::None;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "logs/model_io.log".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
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

    assert!(!replace_delivery_with_latest_tail_read_range_answer(
        &state,
        &task,
        "看下最近 2 行",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
    ));
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("model_io.log 里 error、failed、timeout 各出现 1 次。")
    );
    assert_eq!(
        loop_state.delivery_messages.last().map(String::as_str),
        Some("model_io.log 里 error、failed、timeout 各出现 1 次。")
    );
    assert!(finalizer_summary.is_none());
}

#[test]
fn tail_read_range_observed_answer_preserves_latest_registered_respond() {
    let state = test_state();
    let task = claimed_task("task-tail-preserve-respond");
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = OutputResponseShape::Free;
    route.semantic_kind = crate::OutputSemanticKind::None;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "logs/clawd.run.log".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
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
fn generic_tail_read_does_not_reconstruct_model_summary_from_step_history() {
    let state = test_state();
    let task = claimed_task("task-tail-restore-summary-from-path");
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = OutputResponseShape::OneSentence;
    route.semantic_kind = crate::OutputSemanticKind::None;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "logs/clawd.run.log".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let summary =
        "clawd.run.log 的尾部都是 INFO 级 task_call 流转，整体更像服务正常启动而非刚遇到报错。";
    let mut loop_state = crate::agent_engine::LoopState::new(4);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .delivery_messages
        .push("clawd.run.log".to_string());
    loop_state.last_user_visible_respond = Some("clawd.run.log".to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","mode":"tail","requested_n":20,"path":"/home/guagua/rustclaw/logs/clawd.run.log","excerpt":"1|INFO task_call verifier_result\n2|INFO task_call task_journal_summary"}"#,
    ));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_2", "synthesize_answer", summary));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_3", "respond", summary));
    loop_state.executed_step_results.push(err_step_result(
        "step_4",
        "synthesize_answer",
        "synthesis retry failed",
    ));
    let mut finalizer_summary = None;

    assert!(!replace_delivery_with_latest_tail_read_range_answer(
        &state,
        &task,
        "read the latest log tail and provide the requested takeaway",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
    ));
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("clawd.run.log")
    );
    assert_eq!(
        loop_state.delivery_messages.last().map(String::as_str),
        Some("clawd.run.log")
    );
    assert!(finalizer_summary.is_none());
}

#[test]
fn tail_read_range_observed_answer_replaces_synthesis_after_tail_for_strict_raw_output() {
    let state = test_state();
    let task = claimed_task("task-tail-preserve-synthesis-after-tail");
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = OutputResponseShape::Strict;
    route.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "logs/model_io.log".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
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

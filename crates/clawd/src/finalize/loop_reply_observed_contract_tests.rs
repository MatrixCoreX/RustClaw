use super::*;
use crate::finalize::loop_reply::{
    deterministic_observed_execution_status_summary,
    replace_delivery_with_loop_contract_observed_answer,
    replace_structured_delivery_with_grounded_synthesis,
    replace_structured_delivery_with_grounded_terminal_respond,
};

#[test]
fn direct_structured_observed_answer_prefers_latest_path_result_for_exact_contract() {
    let state = test_state();
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FilePaths;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "plan".to_string();
    route.resolved_intent =
        "If the first plan path is missing, find execution_intent markdown files under plan"
            .to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "system_basic",
        r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":false,"path":"plan/missing.md"}]}"#,
    ));
    loop_state.executed_step_results.push(err_step_result(
        "step_2",
        "read_file",
        "file not found: /home/guagua/rustclaw/plan/missing.md",
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_3",
        "fs_search",
        r#"{"action":"find_name","count":2,"patterns":["execution_intent"],"results":["plan/execution_intent_route_trace_cases.txt","plan/execution_intent_routing_repair_plan_20260509.md"],"root":"plan"}"#,
    ));

    let (answer, summary) =
        direct_structured_observed_answer(Some(&state), &loop_state, Some(&agent_run_context))
            .expect("latest structured path result should answer exact path contract");

    assert!(answer.contains("plan/execution_intent_route_trace_cases.txt"));
    assert!(answer.contains("plan/execution_intent_routing_repair_plan_20260509.md"));
    assert!(!answer.contains("第 1 步"), "answer: {answer}");
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn exact_path_observed_answer_replaces_step_status_after_fallback_success() {
    let state = test_state();
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FilePaths;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "plan".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.executed_step_results.push(err_step_result(
        "step_1",
        "read_file",
        "file not found: /home/guagua/rustclaw/plan/missing.md",
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "fs_search",
        r#"{"action":"find_ext","count":1,"ext":"md","patterns":["execution_intent.md"],"results":["plan/execution_intent_routing_repair_plan_20260509.md"],"root":"plan"}"#,
    ));
    let status_summary = "第 1 步 read_file 失败。第 2 步 fs_search 成功。".to_string();
    loop_state.last_publishable_synthesis_output = Some(status_summary.clone());
    let mut delivery_messages = vec![status_summary];
    let mut finalizer_summary = None;

    prefer_observed_answer_for_exact_contract(
        &state,
        "task-exact-path-fallback",
        &mut loop_state,
        Some(&agent_run_context),
        &mut delivery_messages,
        &mut finalizer_summary,
    );

    assert_eq!(
        delivery_messages,
        vec!["plan/execution_intent_routing_repair_plan_20260509.md".to_string()]
    );
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("plan/execution_intent_routing_repair_plan_20260509.md")
    );
    assert!(
        !delivery_messages[0].contains("第 1 步"),
        "answer: {}",
        delivery_messages[0]
    );
}

#[test]
fn path_locator_observed_answer_replaces_step_status_after_fallback_success() {
    let state = test_state();
    let mut route = free_route_result();
    route.ask_mode = crate::AskMode::planner_execute_plain();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "plan/extra_missing_repair_probe.md".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.executed_step_results.push(err_step_result(
        "step_1",
        "read_file",
        "file not found: /home/guagua/rustclaw/plan/extra_missing_repair_probe.md",
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "fs_search",
        r#"{"action":"find_name","count":2,"patterns":["execution_intent"],"results":["plan/execution_intent_route_trace_cases.txt","plan/execution_intent_routing_repair_plan_20260509.md"],"root":"plan"}"#,
    ));
    let status_summary = "第 1 步 `read_file` 失败。第 2 步 `fs_search` 成功。".to_string();
    loop_state.last_publishable_synthesis_output = Some(status_summary.clone());
    let mut delivery_messages = vec![status_summary];
    let mut finalizer_summary = None;

    prefer_observed_answer_for_exact_contract(
        &state,
        "task-path-locator-fallback",
        &mut loop_state,
        Some(&agent_run_context),
        &mut delivery_messages,
        &mut finalizer_summary,
    );

    assert_eq!(
        delivery_messages,
        vec![
            "plan/execution_intent_route_trace_cases.txt\nplan/execution_intent_routing_repair_plan_20260509.md"
                .to_string()
        ]
    );
    assert!(
        !delivery_messages[0].contains("第 1 步"),
        "answer: {}",
        delivery_messages[0]
    );
}

#[test]
fn strict_existence_path_observed_answer_replaces_step_status_after_fallback_success() {
    let state = test_state();
    let mut route = free_route_result();
    route.ask_mode = crate::AskMode::planner_execute_plain();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "plan/extra_missing_repair_probe.md".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.executed_step_results.push(err_step_result(
        "step_1",
        "read_file",
        "file not found: /home/guagua/rustclaw/plan/extra_missing_repair_probe.md",
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "fs_search",
        r#"{"action":"find_name","count":1,"patterns":["execution_intent.md"],"results":["plan/execution_intent_routing_repair_plan_20260509.md"],"root":"plan"}"#,
    ));
    let status_summary = "第 1 步 `read_file` 失败。第 2 步 `fs_search` 成功。".to_string();
    loop_state.last_publishable_synthesis_output = Some(status_summary.clone());
    let mut delivery_messages = vec![status_summary];
    let mut finalizer_summary = None;

    prefer_observed_answer_for_exact_contract(
        &state,
        "task-strict-existence-path-fallback",
        &mut loop_state,
        Some(&agent_run_context),
        &mut delivery_messages,
        &mut finalizer_summary,
    );

    assert_eq!(
        delivery_messages,
        vec!["plan/execution_intent_routing_repair_plan_20260509.md".to_string()]
    );
    assert!(
        !delivery_messages[0].contains("第 1 步"),
        "answer: {}",
        delivery_messages[0]
    );
}

#[test]
fn scalar_path_observed_answer_replaces_step_status_after_broad_fallback_search() {
    let state = test_state();
    let mut route = free_route_result();
    route.ask_mode = crate::AskMode::planner_execute_plain();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "plan/extra_missing_repair_probe.md".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.executed_step_results.push(err_step_result(
        "step_1",
        "read_file",
        "file not found: /home/guagua/rustclaw/plan/extra_missing_repair_probe.md",
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "fs_search",
        r#"{"action":"find_name","count":2,"patterns":["execution_intent"],"results":["plan/execution_intent_route_trace_cases.txt","plan/execution_intent_routing_repair_plan_20260509.md"],"root":"plan"}"#,
    ));
    let status_summary = "第 1 步 `read_file` 失败。第 2 步 `fs_search` 成功。".to_string();
    loop_state.last_publishable_synthesis_output = Some(status_summary.clone());
    let mut delivery_messages = vec![status_summary];
    let mut finalizer_summary = None;

    prefer_observed_answer_for_exact_contract(
        &state,
        "task-scalar-path-fallback",
        &mut loop_state,
        Some(&agent_run_context),
        &mut delivery_messages,
        &mut finalizer_summary,
    );

    assert!(
        delivery_messages[0].ends_with("plan/execution_intent_routing_repair_plan_20260509.md"),
        "answer: {}",
        delivery_messages[0]
    );
    assert!(
        !delivery_messages[0].contains("第 1 步"),
        "answer: {}",
        delivery_messages[0]
    );
}

#[test]
fn scalar_observed_answer_replaces_run_cmd_step_status_after_fallback_success() {
    let state = test_state();
    let mut route = free_route_result();
    route.ask_mode = crate::AskMode::planner_execute_plain();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    let err = format!(
        "__RC_SKILL_ERROR__:{}",
        serde_json::json!({
            "skill": "run_cmd",
            "error_kind": "nonzero_exit",
            "error_text": "Command failed with exit code 127",
            "platform": "linux",
            "extra": {
                "exit_code": 127,
                "exit_category": "command_not_found",
                "stderr": "missing command",
                "output_truncated": false
            }
        })
    );
    loop_state
        .executed_step_results
        .push(err_step_result("step_1", "run_cmd", &err));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_2", "run_cmd", "/usr/bin/bash\n"));
    let status_summary = "第 1 步 `run_cmd` 失败。第 2 步 `run_cmd` 成功。".to_string();
    loop_state.last_publishable_synthesis_output = Some(status_summary.clone());
    let mut delivery_messages = vec![status_summary];
    let mut finalizer_summary = None;

    prefer_observed_answer_for_exact_contract(
        &state,
        "task-scalar-run-cmd-fallback",
        &mut loop_state,
        Some(&agent_run_context),
        &mut delivery_messages,
        &mut finalizer_summary,
    );

    assert_eq!(delivery_messages, vec!["/usr/bin/bash".to_string()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("/usr/bin/bash")
    );
}

#[test]
fn scalar_raw_command_keeps_written_file_path_synthesis() {
    let state = test_state();
    let mut route = free_route_result();
    route.ask_mode = crate::AskMode::planner_execute_plain();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        "/home/guagua/rustclaw\n",
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "fs_basic",
        r#"{"action":"write_text","path":"/home/guagua/rustclaw/pwd_line_abs.txt"}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_3",
        "synthesize_answer",
        "/home/guagua/rustclaw/pwd_line_abs.txt",
    ));
    let answer = "/home/guagua/rustclaw/pwd_line_abs.txt".to_string();
    loop_state.last_publishable_synthesis_output = Some(answer.clone());
    loop_state
        .output_vars
        .insert("last_written_file_path".to_string(), answer.clone());
    let mut delivery_messages = vec![answer.clone()];
    let mut finalizer_summary = None;

    prefer_observed_answer_for_exact_contract(
        &state,
        "task-written-file-path-synthesis",
        &mut loop_state,
        Some(&agent_run_context),
        &mut delivery_messages,
        &mut finalizer_summary,
    );

    assert_eq!(delivery_messages, vec![answer]);
    assert!(finalizer_summary.is_none());
}

#[test]
fn generated_file_path_report_keeps_plain_written_path_synthesis() {
    let state = test_state();
    let mut route = free_route_result();
    route.ask_mode = crate::AskMode::planner_execute_plain();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::GeneratedFilePathReport;
    route.output_contract.delivery_required = false;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::None;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "pwd_line_abs.txt".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        "/home/guagua/rustclaw\n",
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "fs_basic",
        r#"{"action":"write_text","path":"/home/guagua/rustclaw/pwd_line_abs.txt"}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_3",
        "synthesize_answer",
        "/home/guagua/rustclaw/pwd_line_abs.txt",
    ));
    let answer = "/home/guagua/rustclaw/pwd_line_abs.txt".to_string();
    loop_state.last_publishable_synthesis_output = Some(answer.clone());
    loop_state
        .output_vars
        .insert("last_written_file_path".to_string(), answer.clone());
    let mut delivery_messages = vec![answer.clone()];
    let mut finalizer_summary = None;

    prefer_observed_answer_for_exact_contract(
        &state,
        "task-generated-file-path-report",
        &mut loop_state,
        Some(&agent_run_context),
        &mut delivery_messages,
        &mut finalizer_summary,
    );

    assert_eq!(delivery_messages, vec![answer]);
    assert!(!delivery_messages[0].starts_with("FILE:"));
    assert!(finalizer_summary.is_none());
}

#[test]
fn generated_file_path_report_replaces_write_status_with_written_path() {
    let state = test_state();
    let mut route = free_route_result();
    route.ask_mode = crate::AskMode::planner_execute_plain();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::GeneratedFilePathReport;
    route.output_contract.delivery_required = false;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::None;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "pwd_line_abs.txt".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        "/home/guagua/rustclaw\n",
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "fs_basic",
        "written 21 bytes to /home/guagua/rustclaw/pwd_line_abs.txt",
    ));
    let status = "written 21 bytes to /home/guagua/rustclaw/pwd_line_abs.txt".to_string();
    loop_state
        .executed_step_results
        .push(ok_step_result("step_3", "synthesize_answer", &status));
    let answer = "/home/guagua/rustclaw/pwd_line_abs.txt".to_string();
    loop_state.last_publishable_synthesis_output = Some(status.clone());
    loop_state
        .output_vars
        .insert("last_written_file_path".to_string(), answer.clone());
    let task = claimed_task("task-generated-file-path-report-status");
    let (deterministic, summary) = super::deterministic_matrix_observed_shape_answer(
        &state,
        &task,
        "write path report",
        &loop_state,
        Some(&agent_run_context),
    )
    .expect("deterministic path report answer");
    assert_eq!(deterministic, answer);
    assert!(summary.contract_ok);

    let mut delivery_messages = vec![status];
    let mut finalizer_summary = None;

    prefer_observed_answer_for_exact_contract(
        &state,
        "task-generated-file-path-report-status",
        &mut loop_state,
        Some(&agent_run_context),
        &mut delivery_messages,
        &mut finalizer_summary,
    );

    assert_eq!(delivery_messages, vec![answer]);
    assert!(!delivery_messages[0].starts_with("FILE:"));
}

#[test]
fn generated_file_path_report_prefers_latest_path_synthesis_over_run_cmd_status() {
    let state = test_state();
    let mut route = free_route_result();
    route.ask_mode = crate::AskMode::planner_execute_plain();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::GeneratedFilePathReport;
    route.output_contract.delivery_required = false;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::None;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "pwd_line_abs.txt".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    let status =
        "exit=0 command=echo \"Current working directory: $(pwd)\" > /home/guagua/rustclaw/pwd_line_abs.txt"
            .to_string();
    let answer = "/home/guagua/rustclaw/pwd_line_abs.txt".to_string();
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "run_cmd", &status));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_2", "synthesize_answer", &answer));
    loop_state.last_publishable_synthesis_output = Some(answer.clone());
    let mut delivery_messages = vec![status];
    let mut finalizer_summary = None;

    prefer_observed_answer_for_exact_contract(
        &state,
        "task-generated-file-path-report-run-cmd-status",
        &mut loop_state,
        Some(&agent_run_context),
        &mut delivery_messages,
        &mut finalizer_summary,
    );

    assert_eq!(delivery_messages, vec![answer]);
    assert!(!delivery_messages[0].contains("command="));
    assert!(finalizer_summary.is_some());
}

#[test]
fn loop_contract_scalar_observed_answer_replaces_status_but_keeps_progress() {
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    let mut contract = scalar_route_result().output_contract;
    contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
    loop_state.output_contract = Some(contract);
    loop_state
        .executed_step_results
        .push(err_step_result("step_1", "run_cmd", "command failed"));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_2", "run_cmd", "/usr/bin/bash\n"));
    loop_state.delivery_messages.push(
        "**执行过程**\n1. 调用命令 `missing`\n   错误：\n```text\ncommand failed\n```".to_string(),
    );
    loop_state
        .delivery_messages
        .push("第 1 步 `run_cmd` 失败。第 2 步 `run_cmd` 成功。".to_string());
    let task = claimed_task("task-loop-contract-scalar");
    let mut finalizer_summary = None;

    assert!(replace_delivery_with_loop_contract_observed_answer(
        &task,
        &mut loop_state,
        None,
        &mut finalizer_summary,
    ));

    assert_eq!(loop_state.delivery_messages.len(), 2);
    assert!(loop_state.delivery_messages[0].contains("执行过程"));
    assert_eq!(loop_state.delivery_messages[1], "/usr/bin/bash");
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("/usr/bin/bash")
    );
}

#[test]
fn loop_contract_path_observed_answer_replaces_status_but_keeps_progress() {
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    let mut contract = scalar_route_result().output_contract;
    contract.semantic_kind = crate::OutputSemanticKind::FilePaths;
    loop_state.output_contract = Some(contract);
    loop_state.executed_step_results.push(err_step_result(
        "step_1",
        "read_file",
        "file not found: plan/missing.md",
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "fs_search",
        r#"{"action":"find_ext","count":1,"results":["plan/execution_intent_routing_repair_plan_20260509.md"]}"#,
    ));
    loop_state.delivery_messages.push(
        "**执行过程**\n1. 调用技能 `read_file`\n   错误：\n```text\nfile not found\n```"
            .to_string(),
    );
    loop_state
        .delivery_messages
        .push("Step 1 `read_file` failed. Step 2 `fs_search` succeeded.".to_string());
    let task = claimed_task("task-loop-contract-path");
    let mut finalizer_summary = None;

    assert!(replace_delivery_with_loop_contract_observed_answer(
        &task,
        &mut loop_state,
        None,
        &mut finalizer_summary,
    ));

    assert_eq!(loop_state.delivery_messages.len(), 2);
    assert!(loop_state.delivery_messages[0].contains("执行过程"));
    assert_eq!(
        loop_state.delivery_messages[1],
        "plan/execution_intent_routing_repair_plan_20260509.md"
    );
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("plan/execution_intent_routing_repair_plan_20260509.md")
    );
}

#[test]
fn loop_contract_observed_answer_preserves_explicit_json_delivery() {
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    let mut contract = scalar_route_result().output_contract;
    contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
    loop_state.output_contract = Some(contract);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "system_basic",
        r#"{"path":"/home/guagua/rustclaw/README.md","size_bytes":24929}"#,
    ));
    loop_state
        .delivery_messages
        .push("**执行过程**\n1. 调用技能 `system_basic`".to_string());
    loop_state
        .delivery_messages
        .push(r#"{"path":"/home/guagua/rustclaw/README.md","size_bytes":24929}"#.to_string());
    let task = claimed_task("task-loop-contract-json");
    let mut finalizer_summary = None;

    assert!(!replace_delivery_with_loop_contract_observed_answer(
        &task,
        &mut loop_state,
        None,
        &mut finalizer_summary,
    ));

    assert_eq!(
        loop_state.delivery_messages.last().map(String::as_str),
        Some(r#"{"path":"/home/guagua/rustclaw/README.md","size_bytes":24929}"#)
    );
    assert!(finalizer_summary.is_none());
}

#[test]
fn grounded_terminal_respond_replaces_structured_json_delivery() {
    let task = claimed_task("task-grounded-terminal-respond");
    let mut route = scalar_route_result();
    route.resolved_intent = "extract current working directory".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..crate::agent_engine::AgentRunContext::default()
    };
    let raw = r#"{"arch":"x86_64","cwd":"/home/guagua/rustclaw","workspace_root":"/home/guagua/rustclaw"}"#;
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "system_basic", raw));
    loop_state.delivery_messages.push(raw.to_string());
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 2,
            goal: String::new(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![crate::PlanStep {
                step_id: "step_2".to_string(),
                action_type: "respond".to_string(),
                skill: "respond".to_string(),
                args: serde_json::json!({"content":"/home/guagua/rustclaw"}),
                depends_on: Vec::new(),
                why: String::new(),
            }])),
            verify_result: None,
        });
    let mut finalizer_summary = None;

    assert!(replace_structured_delivery_with_grounded_terminal_respond(
        &task,
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
    ));

    assert_eq!(
        loop_state.delivery_messages,
        vec!["/home/guagua/rustclaw".to_string()]
    );
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("/home/guagua/rustclaw")
    );
    assert_eq!(
        finalizer_summary.and_then(|summary| summary.grounded_ok),
        Some(true)
    );
}

#[test]
fn grounded_latest_synthesis_replaces_structured_json_delivery() {
    let task = claimed_task("task-grounded-synthesis");
    let mut route = scalar_route_result();
    route.resolved_intent = "current system kernel release".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ServiceStatus;
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..crate::agent_engine::AgentRunContext::default()
    };
    let raw = r#"{"system_health":{"kernel_release":"6.17.0-29-generic"},"workspace_root":"/home/guagua/rustclaw"}"#;
    let answer = "6.17.0-29-generic";
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "health_check", raw));
    loop_state.delivery_messages.push(raw.to_string());
    loop_state
        .executed_step_results
        .push(ok_step_result("step_2", "synthesize_answer", answer));
    let mut finalizer_summary = None;

    assert!(replace_structured_delivery_with_grounded_synthesis(
        &task,
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
    ));

    assert_eq!(loop_state.delivery_messages, vec![answer.to_string()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(answer)
    );
    assert_eq!(
        finalizer_summary.and_then(|summary| summary.grounded_ok),
        Some(true)
    );
}

#[test]
fn grounded_terminal_respond_rejects_ungrounded_content() {
    let task = claimed_task("task-grounded-terminal-respond-ungrounded");
    let mut route = scalar_route_result();
    route.resolved_intent = "extract current working directory".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..crate::agent_engine::AgentRunContext::default()
    };
    let raw = r#"{"arch":"x86_64","cwd":"/home/guagua/rustclaw","workspace_root":"/home/guagua/rustclaw"}"#;
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "system_basic", raw));
    loop_state.delivery_messages.push(raw.to_string());
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 2,
            goal: String::new(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![crate::PlanStep {
                step_id: "step_2".to_string(),
                action_type: "respond".to_string(),
                skill: "respond".to_string(),
                args: serde_json::json!({"content":"/tmp/not-observed"}),
                depends_on: Vec::new(),
                why: String::new(),
            }])),
            verify_result: None,
        });
    let mut finalizer_summary = None;

    assert!(!replace_structured_delivery_with_grounded_terminal_respond(
        &task,
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
    ));

    assert_eq!(
        loop_state.delivery_messages.last().map(String::as_str),
        Some(raw)
    );
    assert!(loop_state.last_user_visible_respond.is_none());
    assert!(finalizer_summary.is_none());
}

#[test]
fn loop_contract_observed_answer_requires_contract_evidence_completeness() {
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    let mut contract = scalar_route_result().output_contract;
    contract.response_shape = crate::OutputResponseShape::Scalar;
    contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    loop_state.output_contract = Some(contract);
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "run_cmd", "a short answer\n"));
    loop_state
        .delivery_messages
        .push("Step 1 `run_cmd` succeeded.".to_string());
    let task = claimed_task("task-loop-contract-incomplete-evidence");
    let mut finalizer_summary = None;

    assert!(!replace_delivery_with_loop_contract_observed_answer(
        &task,
        &mut loop_state,
        None,
        &mut finalizer_summary,
    ));

    assert_eq!(
        loop_state.delivery_messages.last().map(String::as_str),
        Some("Step 1 `run_cmd` succeeded.")
    );
    assert!(finalizer_summary.is_none());
}

#[test]
fn loop_contract_observed_answer_requires_matrix_strict_extractor_when_route_is_available() {
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    let mut route = scalar_route_result();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();
    loop_state.output_contract = Some(route.output_contract.clone());
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "unregistered_skill", "3\n"));
    loop_state
        .delivery_messages
        .push("Step 1 `unregistered_skill` succeeded.".to_string());
    let task = claimed_task("task-loop-contract-strict-extractor");
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..crate::agent_engine::AgentRunContext::default()
    };
    let mut finalizer_summary = None;

    assert!(!replace_delivery_with_loop_contract_observed_answer(
        &task,
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
    ));

    assert_eq!(
        loop_state.delivery_messages.last().map(String::as_str),
        Some("Step 1 `unregistered_skill` succeeded.")
    );
    assert!(finalizer_summary.is_none());
}

#[test]
fn loop_contract_observed_answer_does_not_hide_later_failure() {
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    let mut contract = scalar_route_result().output_contract;
    contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
    loop_state.output_contract = Some(contract);
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "run_cmd", "/tmp/value\n"));
    loop_state
        .executed_step_results
        .push(err_step_result("step_2", "run_cmd", "command failed"));
    loop_state
        .delivery_messages
        .push("Step 2 `run_cmd` failed.".to_string());
    let task = claimed_task("task-loop-contract-later-failure");
    let mut finalizer_summary = None;

    assert!(!replace_delivery_with_loop_contract_observed_answer(
        &task,
        &mut loop_state,
        None,
        &mut finalizer_summary,
    ));
    assert_eq!(loop_state.last_user_visible_respond, None);
}

#[test]
fn exact_observed_answer_does_not_replace_mixed_failure_summary() {
    let state = test_state();
    let mut route = free_route_result();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.output_contract.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "run_cmd", "BREAK_A\n"));
    loop_state.executed_step_results.push(err_step_result(
        "step_2",
        "run_cmd",
        "Command failed with exit code 127\nstderr:\nmissing command",
    ));
    let summary =
        "第 1 步 `run_cmd` 成功。第 2 步 `run_cmd` 失败：Command failed with exit code 127。"
            .to_string();
    let mut delivery_messages = vec![summary.clone()];
    let mut finalizer_summary = Some(deterministic_observed_execution_status_summary(&loop_state));

    prefer_observed_answer_for_exact_contract(
        &state,
        "task-exact-observed-mixed-failure",
        &mut loop_state,
        Some(&agent_run_context),
        &mut delivery_messages,
        &mut finalizer_summary,
    );

    assert_eq!(delivery_messages, vec![summary]);
    assert_ne!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("BREAK_A")
    );
}

#[test]
fn scalar_contract_prefers_latest_structured_observed_value_over_planned_delivery() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.delivery_messages.push(
        "true (workspace inherited -- root workspace defines the actual version number)"
            .to_string(),
    );
    loop_state.last_user_visible_respond = loop_state.delivery_messages.last().cloned();
    loop_state.last_publishable_synthesis_output =
        Some("workspace.package.version: 0.1.7".to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "config_basic",
        r#"{"action":"extract_field","exists":true,"field_path":"package.version","format":"toml","resolved_field_path":"package.version","value":{"workspace":true},"value_text":"{\"workspace\":true}","value_type":"object"}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "config_basic",
        r#"{"action":"extract_field","exists":true,"field_path":"workspace.package.version","format":"toml","resolved_field_path":"workspace.package.version","value":"0.1.7","value_text":"0.1.7","value_type":"string"}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_3",
        "synthesize_answer",
        "workspace.package.version: 0.1.7",
    ));
    let mut route = scalar_route_result();
    route.resolved_intent =
        "Read package.version from crates/clawd/Cargo.toml and output only the value.".to_string();
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "crates/clawd/Cargo.toml".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        original_user_request: Some(
            "Read package.version from crates/clawd/Cargo.toml and output only the value."
                .to_string(),
        ),
        ..Default::default()
    };
    let mut finalizer_summary = None;
    let mut delivery = vec![
        "true (workspace inherited -- root workspace defines the actual version number)"
            .to_string(),
    ];
    prefer_observed_answer_for_exact_contract(
        &state,
        "task-1",
        &mut loop_state,
        Some(&agent_run_context),
        &mut delivery,
        &mut finalizer_summary,
    );

    assert_eq!(delivery, vec!["0.1.7".to_string()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("0.1.7")
    );
    assert!(finalizer_summary.is_some());
}

#[test]
fn scalar_contract_replaces_multi_line_read_fields_delivery_with_unique_observed_scalar() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    let polluted_delivery =
        "scripts: {\"build\":\"echo build\",\"dev\":\"echo dev\",\"lint\":\"echo lint\"}\nname: rustclaw-nl-fixture";
    loop_state
        .delivery_messages
        .push(polluted_delivery.to_string());
    loop_state.last_user_visible_respond = loop_state.delivery_messages.last().cloned();
    loop_state.last_publishable_synthesis_output = Some(polluted_delivery.to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "config_basic",
        r#"{"action":"read_fields","path":"/repo/package.json","format":"json","results":[{"exists":true,"field_path":"scripts","resolved_field_path":"scripts","value":{"build":"echo build","dev":"echo dev","lint":"echo lint"},"value_text":"{\"build\":\"echo build\",\"dev\":\"echo dev\",\"lint\":\"echo lint\"}","value_type":"object"},{"exists":true,"field_path":"name","resolved_field_path":"name","value":"rustclaw-nl-fixture","value_text":"rustclaw-nl-fixture","value_type":"string"}]}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "synthesize_answer",
        polluted_delivery,
    ));
    let mut route = scalar_route_result();
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/repo/package.json".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut finalizer_summary = None;
    let mut delivery = vec![polluted_delivery.to_string()];

    prefer_observed_answer_for_exact_contract(
        &state,
        "task-1",
        &mut loop_state,
        Some(&agent_run_context),
        &mut delivery,
        &mut finalizer_summary,
    );

    assert_eq!(delivery, vec!["rustclaw-nl-fixture".to_string()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("rustclaw-nl-fixture")
    );
    assert!(finalizer_summary.is_some());
}

#[test]
fn strict_scalar_count_keeps_planned_explanatory_answer() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "run_cmd", "55\n"));
    loop_state.last_user_visible_respond =
        Some("55 个。当前范围内共有这么多普通文件。".to_string());
    let mut delivery_messages = vec!["55 个。当前范围内共有这么多普通文件。".to_string()];
    let mut route = scalar_route_result();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
    route.output_contract.exact_sentence_count = Some(1);
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut finalizer_summary = None;

    prefer_observed_answer_for_exact_contract(
        &state,
        "task-strict-scalar-count",
        &mut loop_state,
        Some(&agent_run_context),
        &mut delivery_messages,
        &mut finalizer_summary,
    );

    assert_eq!(
        delivery_messages,
        vec!["55 个。当前范围内共有这么多普通文件。"]
    );
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("55 个。当前范围内共有这么多普通文件。")
    );
    assert!(finalizer_summary.is_none());
}

#[test]
fn strict_command_output_summary_prefers_exact_observed_command_output() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    let observed = "/home/guagua/rustclaw\nguagua\nThinkPad-X1\n";
    let synthesis =
        "The current working directory is /home/guagua/rustclaw. The logged-in user is guagua. The hostname is ThinkPad-X1.";
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "run_cmd", observed));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_2", "synthesize_answer", synthesis));
    loop_state.last_publishable_synthesis_output = Some(synthesis.to_string());
    let mut delivery_messages = vec![synthesis.to_string()];
    let mut route = scalar_route_result();
    route.resolved_intent =
        "Run system commands and output each result on a separate line without summary."
            .to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::CommandOutputSummary;
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut finalizer_summary = None;

    prefer_observed_answer_for_exact_contract(
        &state,
        "task-strict-command-output-summary",
        &mut loop_state,
        Some(&agent_run_context),
        &mut delivery_messages,
        &mut finalizer_summary,
    );

    assert_eq!(
        delivery_messages,
        vec!["/home/guagua/rustclaw\nguagua\nThinkPad-X1"]
    );
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("/home/guagua/rustclaw\nguagua\nThinkPad-X1")
    );
    assert_eq!(
        finalizer_summary.and_then(|summary| summary.format_ok),
        Some(true)
    );
}

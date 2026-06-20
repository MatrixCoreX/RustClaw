use super::*;

#[tokio::test]
async fn finalize_loop_reply_preserves_health_check_summary_synthesis() {
    let state = test_state();
    let task = claimed_task("task-service-status-wrapped-health-check");
    let mut route = free_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.resolved_intent = "Show system/service status".to_string();
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ServiceStatus;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        user_request: Some("show status".to_string()),
        ..Default::default()
    };
    let health_output = serde_json::json!({
        "extra": {
            "clawd_health_port_open": true,
            "clawd_log": {
                "exists": true,
                "keyword_error_count": 43
            },
            "clawd_process_count": 1,
            "system_health": {
                "arch": "x86_64",
                "cpu_count": 8,
                "disk_root_available_bytes": 1000,
                "disk_root_total_bytes": 2000,
                "hostname": "rustclaw-host",
                "kernel_release": "6.17.0-test",
                "load_avg_15m": 1.3,
                "load_avg_1m": 1.1,
                "load_avg_5m": 1.2,
                "memory_available_bytes": 3000,
                "memory_total_bytes": 4000,
                "os_family": "linux",
                "service_manager": "systemd",
                "uptime_seconds": 5000,
                "warnings": ["disk_root_low"]
            },
            "telegramd_log": {
                "exists": true,
                "keyword_error_count": 1
            },
            "telegramd_process_count": 0
        },
        "text": "{\"clawd_health_port_open\":true,\"clawd_process_count\":1}"
    })
    .to_string();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "health_check".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(health_output),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let synthesis = concat!(
        "Host rustclaw-host is running on Linux with comfortable memory. ",
        "The main concern is low root disk space. ",
        "clawd is running, while telegramd has no matching process."
    );
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "synthesize_answer".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(synthesis.to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    loop_state.last_publishable_synthesis_output = Some(synthesis.to_string());
    loop_state.last_user_visible_respond = Some(synthesis.to_string());
    loop_state.delivery_messages.push(synthesis.to_string());
    push_raw_plan_text(
        &mut loop_state,
        r#"{"steps":[{"action":"synthesize_answer"},{"action":"respond"}]}"#,
    );

    let reply = finalize_loop_reply(
        &state,
        &task,
        "show status",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should preserve publishable synthesis");

    assert!(!reply.should_fail_task, "reply: {}", reply.text);
    assert!(
        reply
            .text
            .contains("The main concern is low root disk space."),
        "reply: {}",
        reply.text
    );
    assert!(
        !reply.text.contains("system_health.os_family"),
        "reply: {}",
        reply.text
    );
    assert!(
        !reply.text.contains("clawd_health_port_open=true"),
        "reply: {}",
        reply.text
    );
}

#[tokio::test]
async fn finalize_loop_reply_honors_system_health_selector() {
    let state = test_state();
    let task = claimed_task("task-service-status-system-health-selector");
    let mut route = free_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.resolved_intent =
        "Run a basic health check with system_health field selector".to_string();
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ServiceStatus;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route
        .output_contract
        .self_extension
        .structured_field_selector = Some("system_health.*".to_string());
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        user_request: Some("show host OS health fields only".to_string()),
        ..Default::default()
    };
    let health_output = serde_json::json!({
        "extra": {
            "clawd_health_port_open": true,
            "clawd_process_count": 1,
            "system_health": {
                "arch": "x86_64",
                "cpu_count": 8,
                "disk_root_available_bytes": 1000,
                "disk_root_total_bytes": 2000,
                "hostname": "rustclaw-host",
                "kernel_release": "6.17.0-test",
                "load_avg_15m": 1.3,
                "load_avg_1m": 1.1,
                "load_avg_5m": 1.2,
                "memory_available_bytes": 3000,
                "memory_total_bytes": 4000,
                "os_family": "linux",
                "service_manager": "systemd",
                "uptime_seconds": 5000,
                "warnings": ["disk_root_low"]
            },
            "telegramd_process_count": 0
        }
    })
    .to_string();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "health_check", &health_output));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "synthesize_answer",
        "clawd_process_count=1\ntelegramd_process_count=0",
    ));
    loop_state
        .delivery_messages
        .push("clawd_process_count=1\ntelegramd_process_count=0".to_string());

    let reply = finalize_loop_reply(
        &state,
        &task,
        "show host OS health fields only",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should honor system_health selector");

    assert!(!reply.should_fail_task, "reply: {}", reply.text);
    assert!(reply.text.contains("system_health.os_family=linux"));
    assert!(reply
        .text
        .contains("system_health.kernel_release=6.17.0-test"));
    assert!(reply.text.contains("system_health.arch=x86_64"));
    assert!(reply.text.contains("system_health.hostname=rustclaw-host"));
    assert!(reply.text.contains("system_health.service_manager=systemd"));
    assert!(reply.text.contains("system_health.cpu_count=8"));
    assert!(reply.text.contains("system_health.memory_total_bytes=4000"));
    assert!(reply
        .text
        .contains("system_health.memory_available_bytes=3000"));
    assert!(reply
        .text
        .contains("system_health.disk_root_total_bytes=2000"));
    assert!(reply
        .text
        .contains("system_health.disk_root_available_bytes=1000"));
    assert!(reply.text.contains("system_health.load_avg_1m=1.1"));
    assert!(reply.text.contains("system_health.load_avg_5m=1.2"));
    assert!(reply.text.contains("system_health.load_avg_15m=1.3"));
    assert!(reply.text.contains("system_health.uptime_seconds=5000"));
    assert!(reply.text.contains("system_health.warnings=disk_root_low"));
    assert!(
        !reply.text.contains("clawd_process_count"),
        "{}",
        reply.text
    );
    assert!(
        !reply.text.contains("clawd_health_port_open"),
        "{}",
        reply.text
    );
    assert!(
        !reply.text.contains("telegramd_process_count"),
        "{}",
        reply.text
    );
}

#[tokio::test]
async fn finalize_loop_reply_uses_machine_closeout_when_recipe_done_and_synthesis_failed() {
    let state = test_state();
    let task = claimed_task("task-ops-validated-closeout");
    let mut route = free_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.resolved_intent = "Start local service and validate it".to_string();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExecutionFailedStep;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "document/nl_ops_http_demo".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        user_request: Some(
            "When validation passes, explicitly output VALIDATION_PASSED and stop.".to_string(),
        ),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        profile: crate::execution_recipe::ExecutionRecipeProfile::OpsService,
        target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::System,
        phase: crate::execution_recipe::ExecutionRecipePhase::Done,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        saw_mutation: true,
        saw_validation: true,
        ..Default::default()
    };
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        "HTTP_STATUS:200\nVALIDATION_PASSED\n",
    ));
    loop_state.executed_step_results.push(err_step_result(
        "step_2",
        "synthesize_answer",
        "synthesize_answer_failed",
    ));

    let reply = finalize_loop_reply(
        &state,
        &task,
        "启动并验证服务，通过时输出 VALIDATION_PASSED",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should use recipe closeout");

    assert!(!reply.should_fail_task, "reply: {}", reply.text);
    assert!(reply
        .text
        .contains("message_key=clawd.msg.execution_recipe_closeout_system"));
    assert!(reply.text.contains("target_scope=system"));
    assert!(reply.text.contains("profile=ops_service"));
    assert!(reply.text.contains("validation_status=validated"));
    assert!(
        !reply.text.contains("VALIDATION_PASSED"),
        "reply should not infer user-requested marker: {}",
        reply.text
    );
}

#[tokio::test]
async fn finalize_loop_reply_preserves_process_basic_status_summary_synthesis() {
    let state = test_state();
    let task = claimed_task("task-service-status-process-basic-json-synthesis");
    let mut route = free_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.resolved_intent =
        "检查 telegramd 服务进程当前是否仍在运行，并用一句话解释其状态".to_string();
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ServiceStatus;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        user_request: Some(
            "我想确认 telegramd 现在还活着没，你帮我看一下，顺便用一句话解释状态".to_string(),
        ),
        ..Default::default()
    };
    let process_output = serde_json::json!({
        "extra": {
            "action": "ps",
            "exit_code": 0,
            "filter": "telegramd",
            "limit": 200,
            "match_count": 0,
            "output": "exit=0\nPID PPID %CPU %MEM COMM\nno matching processes for filter: telegramd",
            "platform": "linux",
            "process_count": 0,
            "running": false,
            "status": "not_running"
        },
        "text": "exit=0\nPID PPID %CPU %MEM COMM\nno matching processes for filter: telegramd"
    })
    .to_string();
    let status_synthesis =
        "telegramd is not running; the process check found zero matching processes.".to_string();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "process_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(process_output),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "synthesize_answer".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(status_synthesis.clone()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    loop_state.last_publishable_synthesis_output = Some(status_synthesis.clone());
    loop_state.last_user_visible_respond = Some(status_synthesis.clone());
    loop_state.delivery_messages.push(status_synthesis.clone());

    let reply = finalize_loop_reply(
        &state,
        &task,
        "我想确认 telegramd 现在还活着没，你帮我看一下，顺便用一句话解释状态",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should preserve a publishable process status synthesis");

    assert!(!reply.should_fail_task, "reply: {}", reply.text);
    assert_eq!(reply.text, status_synthesis);
    assert!(
        !reply.text.contains("process_basic"),
        "reply: {}",
        reply.text
    );
}

#[tokio::test]
async fn finalize_loop_reply_prefers_service_control_status_over_health_check_dump() {
    let state = test_state();
    let task = claimed_task("task-service-status-service-control-priority");
    let mut route = free_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.resolved_intent = "check ssh service active status in one sentence".to_string();
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ServiceStatus;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        user_request: Some("一句话告诉我 ssh 服务现在是不是 active".to_string()),
        ..Default::default()
    };
    let health_output = serde_json::json!({
        "extra": {
            "clawd_health_port_open": true,
            "clawd_process_count": 1,
            "system_health": {
                "arch": "x86_64",
                "hostname": "rustclaw-host",
                "kernel_release": "6.17.0-test",
                "os_family": "linux",
                "service_manager": "systemd"
            },
            "telegramd_process_count": 0
        },
        "text": "{\"clawd_health_port_open\":true,\"clawd_process_count\":1}"
    })
    .to_string();
    let service_output = serde_json::json!({
        "executed_actions": ["status"],
        "manager_type": "systemd",
        "post_state": "active",
        "pre_state": "active",
        "requested_action": "status",
        "service_name": "sshd",
        "status": "ok",
        "summary": "Status: active",
        "verified": true
    })
    .to_string();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "health_check".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(health_output),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "service_control".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(service_output),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_3".to_string(),
        skill: "synthesize_answer".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some("ssh 服务当前是 active 状态。".to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    loop_state.last_user_visible_respond = Some("ssh 服务当前是 active 状态。".to_string());
    loop_state
        .delivery_messages
        .push("ssh 服务当前是 active 状态。".to_string());

    let reply = finalize_loop_reply(
        &state,
        &task,
        "一句话告诉我 ssh 服务现在是不是 active",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should use service_control evidence");

    assert!(!reply.should_fail_task, "reply: {}", reply.text);
    assert!(
        reply.text.contains("ssh 服务当前是 active 状态。"),
        "publishable synthesis should be preserved: {}",
        reply.text
    );
    assert!(
        !reply.text.contains("system_health.hostname"),
        "service_control evidence should not be replaced by health_check dump: {}",
        reply.text
    );
    assert!(
        !reply.text.contains("service_name=sshd"),
        "one-sentence synthesis should not be replaced with key=value fields: {}",
        reply.text
    );
}

#[tokio::test]
async fn finalize_loop_reply_does_not_infer_service_status_from_raw_systemd_text() {
    let state = test_state();
    let task = claimed_task("task-service-status-raw-systemd-text");
    let mut route = free_route_result();
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ServiceStatus;
    route.output_contract.locator_hint.clear();
    route.output_contract.locator_hint = "telegramd.service".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "run_cmd".to_string(),
        status: StepExecutionStatus::Error,
        output: None,
        error: Some(
            "Command failed with exit code 4\nstderr:\nUnit telegramd.service could not be found."
                .to_string(),
        ),
        started_at: 0,
        finished_at: 0,
    });

    let reply = finalize_loop_reply(
        &state,
        &task,
        "check whether telegramd is running right now and briefly explain the status",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should return a user-visible command result");

    assert!(
        reply.should_fail_task,
        "raw systemd prose should not be promoted to a qualified service-status answer"
    );
    assert!(
        !reply.text.contains("no service unit"),
        "raw text should not trigger local service-status phrase inference: {}",
        reply.text
    );
}

#[tokio::test]
async fn finalize_loop_reply_uses_structured_service_error_kind() {
    let state = test_state();
    let task = claimed_task("task-service-status-structured-missing");
    let mut route = free_route_result();
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ServiceStatus;
    route.output_contract.locator_hint.clear();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let structured_error = serde_json::json!({
        "skill": "service_control",
        "error_kind": "not_found",
        "error_text": "no matching service found for the given target",
        "platform": "linux",
        "manager_type": "unknown",
        "service_name": "telegramd"
    });
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "service_control".to_string(),
        status: StepExecutionStatus::Error,
        output: None,
        error: Some(format!("__RC_SKILL_ERROR__:{structured_error}")),
        started_at: 0,
        finished_at: 0,
    });

    let reply = finalize_loop_reply(
        &state,
        &task,
        "check whether telegramd is running right now and briefly explain the status",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should return a service status answer");

    assert!(!reply.should_fail_task);
    assert!(reply.text.contains("telegramd"));
    assert!(reply.text.contains("not active"));
    assert!(reply.text.contains("no service unit"));
    assert!(!reply.text.contains("__RC_SKILL_ERROR__"));
}

#[tokio::test]
async fn finalize_loop_reply_uses_wrapped_system_basic_info_for_service_status() {
    let state = test_state();
    let task = claimed_task("task-service-status-system-info");
    let mut route = free_route_result();
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ServiceStatus;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let info = serde_json::json!({
        "arch": "x86_64",
        "current_user": "guagua",
        "cwd": "/home/guagua/rustclaw",
        "hostname": "ThinkPad-X1",
        "os": "linux",
        "pid": 2268074,
        "process_rss_bytes": 3084288,
        "uptime_seconds": "868570.44",
        "workspace_root": "/home/guagua/rustclaw"
    });
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            serde_json::json!({
                "extra": info,
                "text": info.to_string()
            })
            .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });

    let reply = finalize_loop_reply(
        &state,
        &task,
        "show status",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should return structured system status fields");

    assert!(!reply.should_fail_task);
    assert!(serde_json::from_str::<serde_json::Value>(&reply.text).is_err());
    assert!(reply.text.contains("ThinkPad-X1"));
    assert!(reply.text.contains("linux"));
    assert!(reply.text.contains("pid=2268074"));
    assert!(reply.text.contains("/home/guagua/rustclaw"));
}

#[test]
fn package_manager_summary_uses_structured_detect_answer() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .output_vars
        .insert("last_skill_name".to_string(), "package_manager".to_string());
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "package_manager".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some("package_manager=brew".to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });

    let mut route = free_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.resolved_intent =
        "check which package manager is recognized and briefly say the everyday default"
            .to_string();
    route.route_reason = "llm_contract:package_manager_detect_summary".to_string();
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let structured_answer =
        direct_structured_observed_answer(None, &loop_state, Some(&agent_run_context));
    assert_eq!(
        structured_answer
            .as_ref()
            .map(|(answer, _summary)| answer.as_str()),
        Some(
            "Detected package manager: brew. Basis: package_manager returned package_manager=brew."
        ),
        "package manager summary should use structured skill evidence"
    );

    assert!(
        direct_non_builtin_skill_raw_answer(&state, &loop_state, Some(&agent_run_context))
            .is_none(),
        "one-sentence summary should not raw-passthrough package_manager output"
    );
}

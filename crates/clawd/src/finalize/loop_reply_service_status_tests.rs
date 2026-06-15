use super::*;

#[tokio::test]
async fn finalize_loop_reply_does_not_replace_health_check_synthesis_with_machine_summary() {
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
                "os_family": "linux",
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
    let bad_synthesis = "clawd status is unclear: health_check did not provide complete clawd_process_count and clawd_health_port_open fields.";
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "synthesize_answer".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(bad_synthesis.to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    loop_state.last_publishable_synthesis_output = Some(bad_synthesis.to_string());
    loop_state.last_user_visible_respond = Some(bad_synthesis.to_string());
    loop_state.delivery_messages.push(bad_synthesis.to_string());
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
    .expect("finalize should return the available synthesized answer");

    assert!(!reply.should_fail_task, "reply: {}", reply.text);
    assert_eq!(reply.text, bad_synthesis);
    assert!(
        !reply.text.contains("health_check.summary"),
        "reply: {}",
        reply.text
    );
    assert!(
        !reply.text.contains("clawd_process_count=1"),
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
async fn finalize_loop_reply_replaces_process_basic_json_synthesis_for_service_status() {
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
    let json_synthesis = serde_json::json!({
        "exit_code": 0,
        "match_count": 0,
        "process_filter": "telegramd",
        "running": false,
        "status": "not_running",
        "status_source": "process_basic.ps",
        "target": "telegramd"
    })
    .to_string();
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
        output: Some(json_synthesis.clone()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    loop_state.last_publishable_synthesis_output = Some(json_synthesis.clone());
    loop_state.last_user_visible_respond = Some(json_synthesis.clone());
    loop_state.delivery_messages.push(json_synthesis);

    let reply = finalize_loop_reply(
        &state,
        &task,
        "我想确认 telegramd 现在还活着没，你帮我看一下，顺便用一句话解释状态",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should replace machine JSON with a service status answer");

    assert!(!reply.should_fail_task, "reply: {}", reply.text);
    assert!(
        serde_json::from_str::<serde_json::Value>(&reply.text).is_err(),
        "service status reply should not expose machine JSON: {}",
        reply.text
    );
    assert!(reply.text.contains("telegramd"), "reply: {}", reply.text);
    assert!(
        reply.text.contains("process_basic"),
        "reply should preserve the observation source: {}",
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

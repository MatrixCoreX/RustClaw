use super::*;

#[tokio::test]
async fn finalize_loop_reply_returns_graceful_result_for_permission_denied_content_evidence() {
    let state = test_state();
    let task = claimed_task("task-content-error-finalize");
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.locator_hint = "/etc/shadow".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.last_user_visible_respond =
        Some("我还没能根据现有证据生成可靠最终答案。".to_string());
    loop_state
        .delivery_messages
        .push("我还没能根据现有证据生成可靠最终答案。".to_string());
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Error,
        output: None,
        error: Some(format!(
            "__RC_SKILL_ERROR__:{}",
            serde_json::json!({
                "skill": "system_basic",
                "error_kind": "permission_denied",
                "error_text": "read_range failed for /etc/shadow",
                "platform": "linux",
                "extra": {
                    "operation": "metadata",
                    "path": "/etc/shadow"
                }
            })
        )),
        started_at: 0,
        finished_at: 0,
    });

    let reply = finalize_loop_reply(
        &state,
        &task,
        "读 /etc/shadow 第一行",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should return a user-visible failure");

    assert!(reply.text.contains("`/etc/shadow`"));
    assert!(reply.text.contains("permission_denied"));
    assert!(!reply.text.contains("__RC_SKILL_ERROR__"));
    assert!(!reply.should_fail_task);
    assert_eq!(reply.messages.len(), 1);
    assert_eq!(reply.messages.last(), Some(&reply.text));
}

#[tokio::test]
async fn finalize_loop_reply_treats_structured_run_cmd_failure_as_user_result() {
    let state = test_state();
    let task = claimed_task("task-structured-run-cmd-nonzero");
    let mut route = free_route_result();
    route.response_shape = OutputResponseShape::OneSentence;
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let structured_error = serde_json::json!({
        "skill": "run_cmd",
        "error_kind": "nonzero_exit",
        "error_text": "Command failed with exit code 7",
        "platform": "linux",
        "extra": {
            "command": "printf problem >&2; exit 7",
            "exit_code": 7,
            "stderr": "problem",
            "output_truncated": false
        }
    });
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(err_step_result(
        "step_1",
        "run_cmd",
        &format!("__RC_SKILL_ERROR__:{structured_error}"),
    ));

    let reply = finalize_loop_reply(
        &state,
        &task,
        "执行命令 printf problem >&2; exit 7，告诉我退出码和错误输出。",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should return a user-visible command failure");

    assert!(!reply.should_fail_task);
    assert!(reply.text.contains("exit_code=7"), "text: {}", reply.text);
    assert!(
        reply.text.contains("stderr=problem"),
        "text: {}",
        reply.text
    );
    assert!(!reply.text.contains("__RC_SKILL_ERROR__"));
    assert_eq!(reply.messages.len(), 1);
    assert_eq!(reply.messages.last(), Some(&reply.text));
}

#[tokio::test]
async fn finalize_loop_reply_sanitizes_contract_rejection_error() {
    let state = test_state();
    let task = claimed_task("task-contract-rejection-sanitized");
    let mut route = free_route_result();
    route.response_shape = OutputResponseShape::OneSentence;
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::ExcerptKindJudgment;
    route.locator_hint = "docs/release_checklist.md".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let structured_error = serde_json::json!({
        "skill": "system_basic",
        "error_kind": "contract_action_rejected",
        "error_text": "action `system_basic.inventory_dir` is rejected by contract `excerpt_kind_judgment`",
        "extra": {
            "action": "system_basic.inventory_dir",
            "contract_match": "excerpt_kind_judgment"
        }
    });
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(err_step_result(
        "step_1",
        "system_basic",
        &format!("__RC_SKILL_ERROR__:{structured_error}"),
    ));

    let reply = finalize_loop_reply(
        &state,
        &task,
        "读取 release_checklist.md 开头并判断它像操作清单还是普通说明",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should return sanitized failure text");

    assert!(reply.text.contains("contract_action_rejected"));
    assert!(!reply.text.contains("__RC_SKILL_ERROR__"));
    assert!(!reply.text.contains("excerpt_kind_judgment"));
    assert!(!reply.text.contains("system_basic.inventory_dir"));
}

use super::*;

#[tokio::test]
async fn content_evidence_step_failure_answer_reports_real_error() {
    let state = test_state();
    let task = claimed_task("task-content-error-direct");
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_hint = "/etc/shadow".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
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

    let (answer, summary) = content_evidence_step_failure_answer(
        &state,
        &task,
        "读 /etc/shadow 第一行",
        &loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("content evidence failure should be publishable");

    assert!(answer.contains("`/etc/shadow`"));
    assert!(answer.to_ascii_lowercase().contains("permission denied"));
    assert_eq!(summary.grounded_ok, Some(true));
    assert_eq!(summary.completion_ok, Some(true));
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[tokio::test]
async fn content_evidence_step_failure_answer_preserves_plan_path_without_locator_hint() {
    let state = test_state();
    let task = claimed_task("task-content-error-plan-target");
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_hint.clear();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        original_user_request: Some("读 /etc/shadow 第一行".to_string()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "read protected file".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![crate::PlanStep {
                step_id: "step_1".to_string(),
                action_type: "call_tool".to_string(),
                skill: "fs_basic".to_string(),
                args: serde_json::json!({
                    "action": "read_range",
                    "path": "/etc/shadow",
                    "mode": "head",
                    "n": 1
                }),
                depends_on: Vec::new(),
                why: String::new(),
            }])),
            verify_result: None,
        });
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: StepExecutionStatus::Error,
        output: None,
        error: Some(format!(
            "__RC_SKILL_ERROR__:{}",
            serde_json::json!({
                "skill": "fs_basic",
                "error_kind": "permission_denied",
                "error_text": "read operation failed: permission denied by the operating system",
                "platform": "linux"
            })
        )),
        started_at: 0,
        finished_at: 0,
    });

    let (answer, summary) = content_evidence_step_failure_answer(
        &state,
        &task,
        "Read the first line of /etc/shadow",
        &loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("content evidence failure should preserve structured plan target");

    assert!(answer.contains("`/etc/shadow`"));
    assert!(answer.contains("permission denied"));
    assert!(answer.contains("locator=`/etc/shadow`"));
    assert!(!answer.contains("`fs_basic` 步骤执行失败"));
    assert_eq!(summary.grounded_ok, Some(true));
    assert_eq!(summary.completion_ok, Some(true));
}

#[tokio::test]
async fn content_evidence_recoverable_crypto_account_error_is_completion() {
    let state = test_state();
    let task = claimed_task("task-crypto-account-error");
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let err = r#"__RC_CRYPTO_ACCOUNT_ACCESS_ERROR__:{"exchange":"binance","detail":"binance error status=401: {\"code\":-2015,\"msg\":\"Invalid API-key, IP, or permissions for action.\"}"}"#;
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .executed_step_results
        .push(err_step_result("step_1", "crypto", err));

    let (answer, summary) = content_evidence_step_failure_answer(
        &state,
        &task,
        "查一下我现在的持仓。",
        &loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("recoverable crypto account error should be publishable");

    assert!(!answer.contains("message_key="));
    assert!(!answer.contains("error_kind="));
    assert!(!answer.contains("__RC_CRYPTO_ACCOUNT_ACCESS_ERROR__"));
    assert!(!answer.trim().is_empty());
    assert_eq!(summary.completion_ok, Some(true));
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[tokio::test]
async fn content_evidence_wrapped_crypto_account_error_is_completion() {
    let state = test_state();
    let task = claimed_task("task-wrapped-crypto-account-error");
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let marker = r#"__RC_CRYPTO_ACCOUNT_ACCESS_ERROR__:{"exchange":"binance","detail":"binance error status=401: {\"code\":-2015,\"msg\":\"Invalid API-key, IP, or permissions for action.\"}"}"#;
    let err = format!(
        "__RC_SKILL_ERROR__:{}",
        serde_json::json!({
            "skill": "crypto",
            "error_kind": "unknown",
            "error_text": marker,
            "extra": null
        })
    );
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .executed_step_results
        .push(err_step_result("step_1", "crypto", &err));

    let (answer, summary) = content_evidence_step_failure_answer(
        &state,
        &task,
        "查一下我现在的持仓。",
        &loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("wrapped recoverable crypto account error should be publishable");

    assert!(!answer.contains("message_key="));
    assert!(!answer.contains("error_kind="));
    assert!(!answer.contains("__RC_CRYPTO_ACCOUNT_ACCESS_ERROR__"));
    assert!(!answer.trim().is_empty());
    assert_eq!(summary.completion_ok, Some(true));
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[tokio::test]
async fn content_evidence_crypto_credential_error_is_completion() {
    let mut state = test_state();
    state.policy.schedule.i18n_dict.insert(
        "crypto.err.okx_not_bound".to_string(),
        "OKX_BINDING_REQUIRED".to_string(),
    );
    let task = claimed_task("task-crypto-credential-error");
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let err = format!(
        "__RC_SKILL_ERROR__:{}",
        serde_json::json!({
            "skill": "crypto",
            "error_kind": "credential_not_bound",
            "error_text": "credential binding unavailable",
            "extra": {
                "error_kind": "credential_not_bound",
                "message_key": "crypto.err.okx_not_bound",
                "exchange": "okx",
                "action": "cancel_all_orders",
                "recoverable": true,
                "status_code": "credential_not_bound"
            }
        })
    );
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .executed_step_results
        .push(err_step_result("step_1", "crypto", &err));

    let (answer, summary) = content_evidence_step_failure_answer(
        &state,
        &task,
        "撤掉所有挂单",
        &loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("recoverable crypto credential error should be publishable");

    assert_eq!(answer, "OKX_BINDING_REQUIRED");
    assert!(!answer.contains("message_key="));
    assert!(!answer.contains("error_kind="));
    assert_eq!(summary.completion_ok, Some(true));
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[tokio::test]
async fn finalize_loop_reply_treats_wrapped_crypto_account_error_as_success() {
    let state = test_state();
    let task = claimed_task("task-finalize-wrapped-crypto-account-error");
    let mut route = free_route_result();
    route.ask_mode = crate::AskMode::planner_execute_with_chat_finalizer();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::MarketQuote;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let marker = r#"__RC_CRYPTO_ACCOUNT_ACCESS_ERROR__:{"exchange":"binance","detail":"binance error status=401: {\"code\":-2015,\"msg\":\"Invalid API-key, IP, or permissions for action.\"}"}"#;
    let err = format!(
        "__RC_SKILL_ERROR__:{}",
        serde_json::json!({
            "skill": "crypto",
            "error_kind": "unknown",
            "error_text": marker,
            "extra": null
        })
    );
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.has_recoverable_failure_context = true;
    loop_state
        .executed_step_results
        .push(err_step_result("step_1", "crypto", &err));
    loop_state.executed_step_results.push(err_step_result(
        "step_2",
        "synthesize_answer",
        "account access explanation was rejected by the content-evidence contract",
    ));

    let reply = finalize_loop_reply(
        &state,
        &task,
        "查一下我现在的持仓。",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should publish recoverable account access result");

    assert!(!reply.should_fail_task);
    assert!(!reply.text.contains("message_key="));
    assert!(!reply.text.contains("error_kind="));
    assert!(!reply.text.contains("__RC_CRYPTO_ACCOUNT_ACCESS_ERROR__"));
    assert!(!reply.text.trim().is_empty());
    assert_eq!(
        reply
            .task_journal
            .as_ref()
            .and_then(|journal| journal.final_status),
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
}

#[tokio::test]
async fn content_evidence_db_query_error_is_completion() {
    let state = test_state();
    let task = claimed_task("task-db-query-error");
    let mut route = free_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "query missing table".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![crate::PlanStep {
                step_id: "step_1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "db_basic".to_string(),
                args: serde_json::json!({
                    "action": "sqlite_query",
                    "db_path": "scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite",
                    "sql": "SELECT * FROM missing_table"
                }),
                depends_on: Vec::new(),
                why: String::new(),
            }])),
            verify_result: None,
        });
    loop_state.executed_step_results.push(err_step_result(
        "step_1",
        "db_basic",
        &format!(
            "__RC_SKILL_ERROR__:{}",
            serde_json::json!({
                "skill": "db_basic",
                "error_kind": "sqlite_query_failed",
                "error_text": "prepare query failed: no such table: missing_table",
                "platform": "linux"
            })
        ),
    ));

    let (answer, summary) = content_evidence_step_failure_answer(
        &state,
        &task,
        "Read missing_table and explain the SQLite error.",
        &loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("db query error should be publishable");

    assert!(answer.contains("missing_table"));
    assert!(answer.contains("no such table"));
    assert_eq!(summary.completion_ok, Some(true));
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[tokio::test]
async fn finalize_loop_reply_treats_missing_read_target_as_user_result() {
    let state = test_state();
    let task = claimed_task("task-missing-read-target");
    let mut route = free_route_result();
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_hint = "document/missing.txt".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Error,
        output: None,
        error: Some(format!(
            "__RC_SKILL_ERROR__:{}",
            serde_json::json!({
                "skill": "system_basic",
                "error_kind": "not_found",
                "error_text": "path was not found: document/missing.txt",
                "platform": "linux",
                "extra": {
                    "operation": "metadata",
                    "path": "document/missing.txt"
                }
            })
        )),
        started_at: 0,
        finished_at: 0,
    });

    let reply = finalize_loop_reply(
        &state,
        &task,
        "读一下 document/missing.txt 开头，然后用一句话总结",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should return a missing-target answer");

    assert!(!reply.should_fail_task);
    assert!(reply.text.contains("clawd.msg.content_missing_target"));
    assert!(reply.text.contains("document/missing.txt"));
    assert_eq!(reply.messages.len(), 1);
    assert_eq!(reply.messages.last(), Some(&reply.text));
    assert_eq!(
        reply
            .task_journal
            .as_ref()
            .and_then(|journal| journal.final_status),
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
}

#[test]
fn content_evidence_missing_target_answer_uses_english_for_non_chinese_request() {
    let state = test_state();
    let task = claimed_task("task-missing-read-target-french");
    let answer = super::content_evidence_missing_target_answer(
        &state,
        &task,
        "Valide plan/does_not_exist_builtin_tool_case.toml comme TOML et explique l'echec clairement.",
        None,
        "__RC_READ_FILE_NOT_FOUND__:plan/does_not_exist_builtin_tool_case.toml",
    );

    assert!(message_has_machine_key(
        &answer,
        "clawd.msg.content_missing_target"
    ));
    assert!(answer.contains("plan/does_not_exist_builtin_tool_case.toml"));
}
#[tokio::test]
async fn missing_read_target_reply_prefers_original_user_language() {
    let state = test_state();
    let mut task = claimed_task("task-missing-read-target-language");
    task.payload_json = serde_json::json!({
        "text": "读取 ./NO_SUCH_RUSTCLAW_TEST_987654.txt 的第一行"
    })
    .to_string();
    let mut route = free_route_result();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_hint = "./NO_SUCH_RUSTCLAW_TEST_987654.txt".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Error,
        output: None,
        error: Some(format!(
            "__RC_SKILL_ERROR__:{}",
            serde_json::json!({
                "skill": "system_basic",
                "error_kind": "not_found",
                "error_text": "path was not found: ./NO_SUCH_RUSTCLAW_TEST_987654.txt",
                "platform": "linux",
                "extra": {
                    "operation": "metadata",
                    "path": "./NO_SUCH_RUSTCLAW_TEST_987654.txt"
                }
            })
        )),
        started_at: 0,
        finished_at: 0,
    });

    let reply = finalize_loop_reply(
        &state,
        &task,
        "Read the first line of the file ./NO_SUCH_RUSTCLAW_TEST_987654.txt.",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should return a missing-target answer");

    assert!(
        reply.text.contains("./NO_SUCH_RUSTCLAW_TEST_987654.txt"),
        "text: {}",
        reply.text
    );
    assert!(!reply.text.contains("未找到"), "text: {}", reply.text);
}

#[tokio::test]
async fn missing_read_target_scalar_contract_keeps_failure_answer_not_path_only() {
    let state = test_state();
    let mut task = claimed_task("task-missing-read-target-scalar");
    task.payload_json = serde_json::json!({
        "text": "读取 ./NO_SUCH_RUSTCLAW_TEST_987654.txt 的第一行"
    })
    .to_string();
    let mut route = scalar_route_result();
    route.resolved_intent =
        "用户请求读取文件 ./NO_SUCH_RUSTCLAW_TEST_987654.txt 的第一行内容。".to_string();
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "./NO_SUCH_RUSTCLAW_TEST_987654.txt".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Error,
        output: None,
        error: Some(format!(
            "__RC_SKILL_ERROR__:{}",
            serde_json::json!({
                "skill": "system_basic",
                "error_kind": "not_found",
                "error_text": "path was not found: ./NO_SUCH_RUSTCLAW_TEST_987654.txt",
                "platform": "linux",
                "extra": {
                    "operation": "metadata",
                    "path": "./NO_SUCH_RUSTCLAW_TEST_987654.txt"
                }
            })
        )),
        started_at: 0,
        finished_at: 0,
    });

    let reply = finalize_loop_reply(
        &state,
        &task,
        "Read the first line of the file ./NO_SUCH_RUSTCLAW_TEST_987654.txt.",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should return a missing-target answer");

    assert!(
        reply.text.contains("./NO_SUCH_RUSTCLAW_TEST_987654.txt"),
        "text: {}",
        reply.text
    );
    assert!(
        reply.text != "./NO_SUCH_RUSTCLAW_TEST_987654.txt",
        "missing target answer must not be reshaped into path-only scalar"
    );
    assert_eq!(reply.messages.len(), 1);
    assert_eq!(reply.messages.last(), Some(&reply.text));
}

#[tokio::test]
async fn finalize_loop_reply_treats_read_file_not_found_marker_as_user_result() {
    let state = test_state();
    let task = claimed_task("task-missing-read-target-marker");
    let mut route = free_route_result();
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_hint = "/tmp/missing.txt".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Error,
        output: None,
        error: Some("__RC_READ_FILE_NOT_FOUND__:/tmp/missing.txt".to_string()),
        started_at: 0,
        finished_at: 0,
    });

    let reply = finalize_loop_reply(
        &state,
        &task,
        "读取 /tmp/missing.txt",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should return a missing-target answer");

    assert!(!reply.should_fail_task);
    assert!(
        reply.text.contains("不存在")
            || reply.text.contains("未找到")
            || reply.text.to_ascii_lowercase().contains("not found")
            || reply.text.to_ascii_lowercase().contains("does not exist")
            || reply.text.contains("clawd.msg.content_missing_target")
    );
    assert_eq!(reply.messages.len(), 1);
    assert_eq!(reply.messages.last(), Some(&reply.text));
    assert_eq!(
        reply
            .task_journal
            .as_ref()
            .and_then(|journal| journal.final_status),
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
}

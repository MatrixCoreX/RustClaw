use super::*;

#[test]
fn backfill_delivery_prefers_contractual_last_respond_over_synthesis() {
    let task = claimed_task("task-contractual-last-respond");
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.last_user_visible_respond = Some("/home/guagua/rustclaw".to_string());
    loop_state.last_publishable_synthesis_output =
        Some("命令执行已完成，但综合答案时出错。".to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        "/home/guagua/rustclaw\n",
    ));
    let mut route = scalar_route_result();
    route.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
    route.locator_hint.clear();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    backfill_delivery_from_last_outputs(&task, &mut loop_state, Some(&ctx));

    assert_eq!(
        loop_state.delivery_messages,
        vec!["/home/guagua/rustclaw".to_string()]
    );
}

#[test]
fn backfill_delivery_accepts_exact_multiline_raw_command_respond() {
    let task = claimed_task("task-contractual-multiline-raw-command");
    let observed = "/home/guagua/rustclaw\nguagua\nThinkPad-X1\n";
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.last_user_visible_respond = Some(observed.trim().to_string());
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "run_cmd", observed));
    let mut route = free_route_result();
    route.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route.response_shape = OutputResponseShape::Free;
    route.requires_content_evidence = true;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    backfill_delivery_from_last_outputs(&task, &mut loop_state, Some(&ctx));

    assert_eq!(
        loop_state.delivery_messages,
        vec![observed.trim().to_string()]
    );
}

#[test]
fn backfill_delivery_uses_free_answer_respond_step() {
    let task = claimed_task("task-free-answer-respond-step");
    let answer =
        "Dry run - RepairEnvelope recovery keeps structured verifier fields and excludes skill text.";
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.last_output = Some(answer.to_string());
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "respond", answer));
    let mut route = free_route_result();
    route.delivery_required = false;
    route.requires_content_evidence = false;
    route.response_shape = OutputResponseShape::Free;
    route.semantic_kind = OutputSemanticKind::None;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    backfill_delivery_from_last_outputs(&task, &mut loop_state, Some(&ctx));

    assert_eq!(loop_state.delivery_messages, vec![answer.to_string()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(answer)
    );
}

#[test]
fn backfill_delivery_defers_structured_dry_run_payload_to_finalizer_projection() {
    let task = claimed_task("task-dry-run-projection-defer");
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.last_publishable_synthesis_output = Some(
        "Dry-run summary: provider minimax, model speech-2.8-turbo, output path present."
            .to_string(),
    );
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "audio_synthesize",
        r#"{"text":"AUDIO_SYNTHESIZE_DRY_RUN","extra":{"dry_run":true,"provider":"minimax","model":"speech-2.8-turbo","model_kind":"dry_run","output_path":"/home/guagua/rustclaw/document/media_dry_run/audio_check.mp3","planned_outputs":[{"type":"audio_file","path":"/home/guagua/rustclaw/document/media_dry_run/audio_check.mp3"}],"outputs":[]}}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "synthesize_answer",
        "Dry-run summary: provider minimax, model speech-2.8-turbo, output path present.",
    ));
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = OutputResponseShape::Strict;
    route.delivery_required = false;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    backfill_delivery_from_last_outputs(&task, &mut loop_state, Some(&ctx));

    assert!(loop_state.delivery_messages.is_empty());
}

#[test]
fn backfill_delivery_accepts_strict_json_projection_marker() {
    let task = claimed_task("task-strict-json-projection");
    let answer = r#"{"created_files":["/workspace/calc_core.py"],"test_command":"python3 test_calc_core.py","test_status":"passed"}"#;
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.last_publishable_synthesis_output = Some(answer.to_string());
    loop_state.output_vars.insert(
        "agent_loop.strict_json_projection_publishable".to_string(),
        "true".to_string(),
    );
    loop_state.output_vars.insert(
        "agent_loop.strict_json_projection_output".to_string(),
        answer.to_string(),
    );
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        "All tests passed.\n",
    ));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_2", "synthesize_answer", answer));

    assert_eq!(
        crate::finalize::loop_reply::valid_publishable_synthesis_output(&loop_state),
        Some(answer)
    );

    backfill_delivery_from_last_outputs(&task, &mut loop_state, None);

    assert_eq!(loop_state.delivery_messages, vec![answer.to_string()]);
}

#[test]
fn backfill_delivery_uses_terminal_contract_respond_without_observed_execution() {
    let task = claimed_task("task-terminal-contract-respond-no-observed-execution");
    let answer = r#"Dry-run async_start contract shape:

{
  "args": {
    "command": "sleep 2 && echo RUSTCLAW_ASYNC_100",
    "async_start": true,
    "poll_after_seconds": 5
  },
  "extra": {
    "adapter_kind": "local_process_poll",
    "checkpoint_id": "ckpt-<runtime-uuid>",
    "poll_ref": "run_cmd:local_process_poll:ckpt-<runtime-uuid>",
    "cancel_ref": "run_cmd:cancel:ckpt-<runtime-uuid>"
  }
}"#;
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.last_output = Some(answer.to_string());
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "respond", answer));
    let mut route = free_route_result();
    route.delivery_required = false;
    route.requires_content_evidence = false;
    route.response_shape = OutputResponseShape::Free;
    route.semantic_kind = OutputSemanticKind::None;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    backfill_delivery_from_last_outputs(&task, &mut loop_state, Some(&ctx));

    assert_eq!(loop_state.delivery_messages, vec![answer.to_string()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(answer)
    );
}

#[test]
fn backfill_delivery_does_not_use_respond_step_for_content_evidence_route() {
    let task = claimed_task("task-content-evidence-respond-step");
    let answer = "Dry run - content evidence routes must not backfill from a plain respond step.";
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.last_output = Some(answer.to_string());
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "respond", answer));
    let mut route = free_route_result();
    route.delivery_required = false;
    route.requires_content_evidence = true;
    route.response_shape = OutputResponseShape::Strict;
    route.semantic_kind = OutputSemanticKind::None;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    backfill_delivery_from_last_outputs(&task, &mut loop_state, Some(&ctx));

    assert!(loop_state.delivery_messages.is_empty());
}

#[test]
fn backfill_delivery_prefers_content_evidence_synthesis_over_locator_respond() {
    let task = claimed_task("task-content-evidence-synthesis-over-locator");
    let synthesis = "The query returned rows from the orders table.";
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.last_user_visible_respond = Some("test_contract.sqlite".to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "db_basic",
        r#"{"extra":{"action":"sqlite_query","db_path":"/tmp/test_contract.sqlite","result":{"columns":["id"],"rows":[{"id":1}]}},"text":"test_contract.sqlite"}"#,
    ));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_2", "synthesize_answer", synthesis));
    let mut route = free_route_result();
    route.delivery_required = false;
    route.requires_content_evidence = true;
    route.response_shape = OutputResponseShape::Free;
    route.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    backfill_delivery_from_last_outputs(&task, &mut loop_state, Some(&ctx));

    assert_eq!(loop_state.delivery_messages, vec![synthesis.to_string()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(synthesis)
    );
}

#[tokio::test]
async fn finalize_loop_reply_keeps_exact_single_line_observed_respond() {
    let state = test_state();
    let task = claimed_task("task-single-line-observed-respond");
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.last_user_visible_respond = Some("/home/guagua/rustclaw".to_string());
    loop_state.last_publishable_synthesis_output =
        Some("执行成功了，但合成最终答案的环节遇到问题。".to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        "/home/guagua/rustclaw\n",
    ));
    loop_state.executed_step_results.push(err_step_result(
        "step_2",
        "synthesize_answer",
        "synthesis failed",
    ));
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    let reply = finalize_loop_reply(
        &state,
        &task,
        "执行命令 pwd，直接回复执行结果，不要解释",
        loop_state,
        Some(&ctx),
    )
    .await
    .expect("finalize should succeed");

    assert_eq!(reply.text, "/home/guagua/rustclaw");
    assert!(!reply.should_fail_task);
    assert_eq!(
        reply.messages.last().map(String::as_str),
        Some("/home/guagua/rustclaw")
    );
    assert_eq!(reply.messages, vec!["/home/guagua/rustclaw".to_string()]);
    assert!(reply
        .messages
        .iter()
        .all(|message| !crate::finalize::is_execution_summary_message(message)));
}

#[tokio::test]
async fn finalize_loop_reply_keeps_exact_multiline_raw_command_observed_respond() {
    let state = test_state();
    let task = claimed_task("task-multiline-raw-command-observed-respond");
    let observed = "/home/guagua/rustclaw\nguagua\nThinkPad-X1\n";
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.last_user_visible_respond = Some(observed.trim().to_string());
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "run_cmd", observed));
    let mut route = free_route_result();
    route.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route.response_shape = OutputResponseShape::Strict;
    route.requires_content_evidence = true;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    let reply = finalize_loop_reply(
        &state,
        &task,
        "pwd whoami hostname 三个结果每个一行 不要总结",
        loop_state,
        Some(&ctx),
    )
    .await
    .expect("finalize should keep exact raw command output");

    assert_eq!(reply.text, observed.trim());
    assert!(!reply.should_fail_task);
    assert_eq!(reply.messages, vec![observed.trim().to_string()]);
    assert!(!reply.is_llm_reply);
}

#[tokio::test]
async fn finalize_loop_reply_uses_publishable_synthesis_output() {
    let state = test_state();
    let task = claimed_task("task-synth-finalize");
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "run_cmd".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some("rustclaw.service".to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "synthesize_answer".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some("有，路径：/tmp/rustclaw.service".to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    loop_state.last_publishable_synthesis_output =
        Some("有，路径：/tmp/rustclaw.service".to_string());
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(scalar_route_result()),
        ..Default::default()
    };

    let reply = finalize_loop_reply(
        &state,
        &task,
        "检查 rustclaw.service 是否存在并给出路径",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should succeed");

    assert_eq!(reply.text, "有，路径：/tmp/rustclaw.service");
    assert_eq!(reply.messages, vec!["有，路径：/tmp/rustclaw.service"]);
    assert!(!reply.should_fail_task);
    assert!(!reply.is_llm_reply);
}

#[tokio::test]
async fn finalize_loop_reply_prefers_synthesis_over_raw_delivery_listing() {
    let state = test_state();
    let task = claimed_task("task-synth-over-raw-listing");
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    let raw_listing = "Untitled\nauth-key.sh\ncheck_no_nl_hardmatch.py\nnl_tests\n";
    let synthesis = "该 scripts 目录主要包含用于测试、回归、代码检查和运行时验证的脚本。";
    loop_state.has_tool_or_skill_output = true;
    loop_state.delivery_messages.push(raw_listing.to_string());
    loop_state.last_user_visible_respond = Some(raw_listing.to_string());
    loop_state.last_publishable_synthesis_output = Some(synthesis.to_string());
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "run_cmd".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(raw_listing.to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "synthesize_answer".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(synthesis.to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let mut route = free_route_result();
    route.response_shape = OutputResponseShape::OneSentence;
    route.requires_content_evidence = true;
    route.semantic_kind = OutputSemanticKind::RawCommandOutput;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    let reply = finalize_loop_reply(
        &state,
        &task,
        "执行 ls scripts，然后用一句话告诉我这个目录大概放的是什么",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should succeed");

    assert_eq!(reply.text, synthesis);
    assert_eq!(reply.messages, vec![synthesis.to_string()]);
    assert!(!reply.should_fail_task);
}

#[tokio::test]
async fn finalize_loop_reply_prefers_latest_synthesis_for_compound_observations() {
    let state = test_state();
    let task = claimed_task("task-compound-observation-synth");
    let partial_table = "| name | score |\n| --- | --- |\n| beta | 12 |";
    let synthesis = "Log section reports warn=2 and error=1. Document section reports Service Notes. Markdown table ranks beta above gamma and alpha.";
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.delivery_messages.push(partial_table.to_string());
    loop_state.last_user_visible_respond = Some(partial_table.to_string());
    loop_state.last_publishable_synthesis_output = Some(partial_table.to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "log_analyze",
        r#"{"keyword_counts":{"warn":2,"error":1},"path":"logs/app.log"}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "doc_parse",
        r##"{"extra":{"content_excerpt":"# Service Notes\nbody","path":"docs/service_notes.md"}}"##,
    ));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_3", "synthesize_answer", synthesis));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_4", "respond", partial_table));
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.semantic_kind = OutputSemanticKind::None;
    route.response_shape = OutputResponseShape::Strict;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    let reply = finalize_loop_reply(
        &state,
        &task,
        "compound observation",
        loop_state,
        Some(&ctx),
    )
    .await
    .expect("finalize should succeed");

    assert_eq!(reply.text, synthesis);
    assert_eq!(reply.messages, vec![synthesis.to_string()]);
    assert!(!reply.should_fail_task);
}

#[tokio::test]
async fn finalize_loop_reply_prefers_content_excerpt_synthesis_over_title_delivery() {
    let state = test_state();
    let task = claimed_task("task-content-excerpt-title-delivery");
    let synthesis = "文件存在；读取到 20 行；标题中出现 RustClaw。";
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.has_tool_or_skill_output = true;
    loop_state.delivery_messages.push("README.md".to_string());
    loop_state.last_user_visible_respond = Some("README.md".to_string());
    loop_state.last_publishable_synthesis_output = Some(synthesis.to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "doc_parse",
        r##"{"extra":{"action":"parse_doc","content_excerpt":"# RustClaw\n\nRustClaw runtime.","path":"README.md","metadata":{"title":"README.md"}},"text":"README.md"}"##,
    ));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_2", "synthesize_answer", synthesis));
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.response_shape = OutputResponseShape::Strict;
    route.delivery_required = false;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    assert_eq!(
        crate::finalize::loop_reply::valid_publishable_synthesis_output(&loop_state),
        Some(synthesis)
    );
    assert!(
        crate::finalize::loop_reply::route_expects_synthesis_over_raw_observation(
            ctx.output_contract.as_ref().expect("output contract")
        )
    );

    let reply = finalize_loop_reply(
        &state,
        &task,
        "读取 README.md 前 20 行并回答标题是否包含 RustClaw",
        loop_state,
        Some(&ctx),
    )
    .await
    .expect("finalize should keep content excerpt synthesis");

    assert_eq!(reply.text, synthesis);
    assert_eq!(reply.messages, vec![synthesis.to_string()]);
    assert!(!reply.should_fail_task);
}

#[tokio::test]
async fn finalize_loop_reply_prefers_content_excerpt_respond_synthesis_over_title_delivery() {
    let state = test_state();
    let task = claimed_task("task-content-excerpt-respond-title-delivery");
    let synthesis =
        "1. 文件是否存在：是。\n2. 读取到的行数：20 行。\n3. 标题中是否出现 RustClaw：是。";
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.has_tool_or_skill_output = true;
    loop_state.delivery_messages.push("README.md".to_string());
    loop_state.last_user_visible_respond = Some("README.md".to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r##"{"extra":{"action":"read_range","end_line":20,"excerpt":"1|# RustClaw\n2|\n3|body","path":"/home/guagua/rustclaw/README.md","resolved_path":"/home/guagua/rustclaw/README.md","start_line":1,"total_lines":806},"text":"{\"action\":\"read_range\"}"}"##,
    ));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_2", "respond", synthesis));
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.semantic_kind = OutputSemanticKind::None;
    route.response_shape = OutputResponseShape::Strict;
    route.delivery_required = false;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    let reply = finalize_loop_reply(
        &state,
        &task,
        "读取 README.md 前 20 行并回答标题是否包含 RustClaw",
        loop_state,
        Some(&ctx),
    )
    .await
    .expect("finalize should keep respond-form content excerpt synthesis");

    assert_eq!(reply.text, synthesis);
    assert_eq!(reply.messages, vec![synthesis.to_string()]);
    assert!(!reply.should_fail_task);
}

#[tokio::test]
async fn finalize_loop_reply_prefers_db_rows_synthesis_over_locator_title_delivery() {
    let state = test_state();
    let task = claimed_task("task-db-rows-synthesis-over-locator-title");
    let synthesis = "对 test_contract.sqlite 执行的只读查询结果如下：\n\n- 被查询表：按名称排序的第一个表 `orders`\n- SQL：`SELECT * FROM orders LIMIT 5;`（仅 SELECT，未执行写入语句）\n- 共返回 2 行，列结构为 `id, user_id, amount, status`\n\n具体行内容：\n\n| id | user_id | amount | status |\n|----|---------|--------|--------|\n| 1  | 1       | 19.9   | paid   |\n| 2  | 2       | 42.5   | pending |\n\n表不为空，行数已少于 5 行上限，因此无需截断。整个过程仅使用只读 SELECT，未触发任何写入 SQL。";
    let mut loop_state = crate::agent_engine::LoopState::new(4);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .delivery_messages
        .push("test_contract.sqlite".to_string());
    loop_state.last_user_visible_respond = Some("test_contract.sqlite".to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "db_basic",
        r#"{"extra":{"action":"list_tables","db_path":"/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite","field_value":{"table_count":3,"tables":["orders","service_logs","users"]},"result":{"columns":["name"],"rows":[{"name":"orders"},{"name":"service_logs"},{"name":"users"}]},"table_count":3,"tables":["orders","service_logs","users"]},"text":"{\"columns\":[\"name\"],\"rows\":[{\"name\":\"orders\"},{\"name\":\"service_logs\"},{\"name\":\"users\"}]}"}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "db_basic",
        r#"{"extra":{"action":"sqlite_query","db_path":"/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite","result":{"columns":["id","user_id","amount","status"],"rows":[{"amount":19.9,"id":1,"status":"paid","user_id":1},{"amount":42.5,"id":2,"status":"pending","user_id":2}]},"sql":"SELECT * FROM orders LIMIT 5;"},"text":"{\"columns\":[\"id\",\"user_id\",\"amount\",\"status\"],\"rows\":[{\"amount\":19.9,\"id\":1,\"status\":\"paid\",\"user_id\":1},{\"amount\":42.5,\"id\":2,\"status\":\"pending\",\"user_id\":2}]}"}"#,
    ));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_3", "synthesize_answer", synthesis));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_4", "respond", synthesis));
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.response_shape = OutputResponseShape::Free;
    route.delivery_required = false;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint =
        "/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite"
            .to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    let reply = finalize_loop_reply(&state, &task, "query database rows", loop_state, Some(&ctx))
        .await
        .expect("finalize should keep db rows synthesis");

    assert_eq!(reply.text, synthesis);
    assert_eq!(reply.messages, vec![synthesis.to_string()]);
    assert!(!reply.should_fail_task);
}

#[tokio::test]
async fn finalize_loop_reply_uses_multi_locator_route_for_compound_synthesis() {
    let state = test_state();
    let task = claimed_task("task-compound-route-locator-synth");
    let partial_table = "| name | score |\n| --- | --- |\n| beta | 12 |";
    let synthesis = "### 1. Log analysis\n\nWARN count is 2 and ERROR count is 1.\n\n### 2. Service Notes\n\nThe document describes a small control panel and troubleshooting order.\n\n### 3. Markdown table\n\n| name | score |\n| --- | --- |\n| beta | 12 |\n| gamma | 9 |\n| alpha | 7 |";
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.delivery_messages.push(partial_table.to_string());
    loop_state.last_user_visible_respond = Some(partial_table.to_string());
    loop_state.last_publishable_synthesis_output = Some(partial_table.to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "log_analyze",
        r#"{"keyword_counts":{"warn":2,"error":1},"path":"logs/app.log"}"#,
    ));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_2", "synthesize_answer", synthesis));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_3", "respond", partial_table));
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.semantic_kind = OutputSemanticKind::CommandOutputSummary;
    route.response_shape = OutputResponseShape::Free;
    route.locator_hint = "logs/app.log | docs/service_notes.md".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    let reply = finalize_loop_reply(
        &state,
        &task,
        "compound observation",
        loop_state,
        Some(&ctx),
    )
    .await
    .expect("finalize should succeed");

    assert_eq!(reply.text, synthesis);
    assert_eq!(reply.messages, vec![synthesis.to_string()]);
    assert!(!reply.should_fail_task);
}

#[tokio::test]
async fn finalize_loop_reply_replaces_raw_read_delivery_with_latest_synthesis() {
    let state = test_state();
    let task = claimed_task("task-raw-read-delivery-synthesis");
    let raw_read = r#"{"action":"read_range","mode":"head","excerpt":"1|alpha\n2|beta\n3|gamma","path":"/tmp/app.log"}"#;
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "fs_basic", raw_read));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "synthesize_answer",
        "검색 결과 없음",
    ));
    loop_state.delivery_messages.push(raw_read.to_string());
    loop_state.last_user_visible_respond = Some(raw_read.to_string());
    loop_state.last_publishable_synthesis_output = Some("검색 결과 없음".to_string());
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    let reply = finalize_loop_reply(
        &state,
        &task,
        "app.log 에서 impossible_keyword_987 을 찾아보고 결과를 짧게 말해.",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should use synthesis");

    assert_eq!(reply.text, "검색 결과 없음");
    assert_eq!(reply.messages, vec!["검색 결과 없음".to_string()]);
    assert!(!reply.should_fail_task);
}

#[tokio::test]
async fn finalize_loop_reply_prefers_exact_sentence_synthesis_over_raw_read() {
    let state = test_state();
    let task = claimed_task("task-exact-sentence-read-summary");
    let raw_read = serde_json::json!({
        "extra": {
            "action": "read_range",
            "mode": "head",
            "requested_n": 20,
            "path": "README.md",
        },
        "text": "# RustClaw\n\nRustClaw is a local Rust agent runtime centered on clawd.",
    })
    .to_string();
    let synthesis =
        "RustClaw is a local Rust agent runtime. It is centered on clawd. It supports channel and skill execution.";
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "fs_basic", &raw_read));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_2", "synthesize_answer", synthesis));
    loop_state.delivery_messages.push(raw_read);
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = OutputResponseShape::Strict;
    route.exact_sentence_count = Some(3);
    route.semantic_kind = OutputSemanticKind::None;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    let reply = finalize_loop_reply(
        &state,
        &task,
        "read README head and summarize in three sentences",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should use exact-sentence synthesis");

    assert_eq!(reply.text, synthesis);
    assert_eq!(reply.messages, vec![synthesis.to_string()]);
    assert!(!reply.should_fail_task);
}

#[tokio::test]
async fn finalize_loop_reply_keeps_strict_raw_tail_read_delivery_over_synthesis() {
    let state = test_state();
    let task = claimed_task("task-strict-raw-tail-keeps-observed");
    let observed = "98|first observed line\n99|second observed line";
    let raw_read = serde_json::json!({
        "extra": {
            "action": "read_range",
            "mode": "tail",
            "requested_n": 2,
            "path": "/tmp/app.log",
            "resolved_path": "/tmp/app.log",
            "excerpt": observed
        },
        "text": serde_json::json!({
            "action": "read_range",
            "mode": "tail",
            "excerpt": observed
        })
        .to_string()
    })
    .to_string();
    let synthesis = "planned fallback text";
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "fs_basic", &raw_read));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_2", "synthesize_answer", synthesis));
    loop_state.delivery_messages.push(observed.to_string());
    loop_state.last_user_visible_respond = Some(observed.to_string());
    loop_state.last_publishable_synthesis_output = Some(synthesis.to_string());
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route.response_shape = OutputResponseShape::Strict;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    let reply = finalize_loop_reply(
        &state,
        &task,
        "show the last two lines exactly",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should preserve strict raw read output");

    let normalized_observed = "first observed line\nsecond observed line";
    assert_eq!(reply.text, normalized_observed);
    assert_eq!(reply.messages, vec![normalized_observed.to_string()]);
    assert!(!reply.should_fail_task);
}

#[tokio::test]
async fn finalize_loop_reply_uses_latest_fs_basic_path_fact_after_repair() {
    let state = test_state();
    let task = claimed_task("task-path-fact-after-repair");
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"path_batch_facts","count":4,"facts":[{"exists":false,"path":"agent_guard.toml"},{"exists":false,"path":"audio.toml"},{"exists":false,"path":"browser_web_wait_map.json"},{"exists":false,"path":"channel_commands.toml"}],"include_missing":true}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "fs_basic",
        r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":true,"fact":{"kind":"dir","path":"configs/channels","resolved_path":"/tmp/repo/configs/channels","size_bytes":4096},"path":"/tmp/repo/configs/channels"}],"include_missing":true}"#,
    ));
    let mut route = free_route_result();
    route.response_shape = OutputResponseShape::Strict;
    route.requires_content_evidence = true;
    route.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "/tmp/repo/configs/channels".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    let reply = finalize_loop_reply(
        &state,
        &task,
        "看最后一个的基本信息，只回答路径和类型",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should succeed");

    assert!(reply
        .text
        .contains("message_key=clawd.msg.path_fact.observed"));
    assert!(reply.text.contains("reason_code=path_fact_observed"));
    assert!(reply.text.contains("exists=true"));
    assert!(reply.text.contains("path=/tmp/repo/configs/channels"));
    assert!(reply.text.contains("kind=dir"));
    assert!(!reply.text.contains("没能整理成可靠结论"));
    assert!(reply
        .messages
        .iter()
        .all(|message| !crate::finalize::is_execution_summary_message(message)));
    assert_eq!(
        reply.messages.last().map(String::as_str),
        Some(reply.text.as_str())
    );
    assert!(!reply.should_fail_task);
    assert!(!reply.is_llm_reply);
}

#[tokio::test]
async fn finalize_loop_reply_prefers_synthesis_over_raw_last_respond() {
    let state = test_state();
    let task = claimed_task("task-synth-over-raw");
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .output_vars
        .insert("last_skill_name".to_string(), "git_basic".to_string());
    let raw_git = "exit=0\nabc123 fix deployment docs\n";
    loop_state.last_user_visible_respond = Some(raw_git.to_string());
    loop_state.last_publishable_synthesis_output =
        Some("RustClaw 的部署可按项目文档和安装脚本完成。".to_string());
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "git_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(raw_git.to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "synthesize_answer".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some("RustClaw 的部署可按项目文档和安装脚本完成。".to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    let reply = finalize_loop_reply(
        &state,
        &task,
        "帮我写一段 RustClaw 部署说明",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should succeed");

    assert_eq!(reply.text, "RustClaw 的部署可按项目文档和安装脚本完成。");
    assert_eq!(
        reply.messages.last().map(String::as_str),
        Some("RustClaw 的部署可按项目文档和安装脚本完成。")
    );
    assert!(reply
        .messages
        .iter()
        .all(|message| !crate::finalize::is_execution_summary_message(message)));
}

#[tokio::test]
async fn finalize_loop_reply_keeps_article_synthesis_after_repair_success() {
    let state = test_state();
    let task = claimed_task("task-synth-after-repair");
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "list_dir".to_string(),
        status: StepExecutionStatus::Error,
        output: None,
        error: Some("file operation failed: target path was not found".to_string()),
        started_at: 0,
        finished_at: 0,
    });
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "read_file".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some("# RustClaw\n\nRustClaw is a local Rust agent runtime.".to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let article = "RustClaw 是一个本地优先的 Rust 智能体运行时，围绕 clawd、技能调度和多渠道入口组织，可用于通过聊天或浏览器完成项目管理与自动化任务。".to_string();
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_3".to_string(),
        skill: "synthesize_answer".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(article.clone()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    loop_state.delivery_messages.push(
        "**执行过程**\n1. 调用技能 `list_dir`\n   错误：\n```text\nfile operation failed: target path was not found\n```"
            .to_string(),
    );
    loop_state.delivery_messages.push(article.clone());
    loop_state.last_user_visible_respond = Some(article.clone());
    loop_state.last_publishable_synthesis_output = Some(article.clone());
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    let reply = finalize_loop_reply(
        &state,
        &task,
        "帮我写一篇关于 RustClaw 的长文",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should succeed");

    assert_eq!(reply.text, article);
    assert_eq!(
        reply.messages.last().map(String::as_str),
        Some(article.as_str())
    );
    assert!(
        !reply.text.contains("第 1 步"),
        "article synthesis must not be replaced by step status: {}",
        reply.text
    );
}

#[tokio::test]
async fn finalize_loop_reply_replaces_template_placeholder_with_synthesis() {
    let state = test_state();
    let task = claimed_task("task-synth-placeholder");
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .delivery_messages
        .push("{{synthesized}}".to_string());
    loop_state.last_user_visible_respond = Some("{{synthesized}}".to_string());
    loop_state.last_publishable_synthesis_output =
        Some("RustClaw 可以按 README 中的安装脚本路径完成部署。".to_string());
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "read_file".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some("# RustClaw\n\nUse install-rustclaw-cmd.sh".to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "synthesize_answer".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some("RustClaw 可以按 README 中的安装脚本路径完成部署。".to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    let reply = finalize_loop_reply(
        &state,
        &task,
        "帮我写一段 RustClaw 部署说明",
        loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("finalize should succeed");

    assert_eq!(
        reply.text,
        "RustClaw 可以按 README 中的安装脚本路径完成部署。"
    );
    assert_eq!(
        reply.messages.last().map(String::as_str),
        Some("RustClaw 可以按 README 中的安装脚本路径完成部署。")
    );
    assert!(!reply.text.contains("{{"));
}

use super::*;
use crate::finalize::loop_reply::successful_content_observation_should_precede_status_summary;

#[test]
fn deterministic_observed_execution_status_answer_reports_mixed_results() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "health_check",
        r#"{"ok":true}"#,
    ));
    loop_state.executed_step_results.push(err_step_result(
        "step_2",
        "run_cmd",
        "Command failed with exit code 127\nstderr:\nmissing command",
    ));

    let answer = deterministic_observed_execution_status_answer(
        &state,
        "先检查健康，再执行缺失命令，然后总结哪一步成功了、哪一步失败了。",
        &loop_state,
    )
    .expect("mixed observed results should produce deterministic answer");

    assert!(answer.contains("step.1.skill=health_check"));
    assert!(answer.contains("step.1.status=ok"));
    assert!(answer.contains("step.2.skill=run_cmd"));
    assert!(answer.contains("step.2.status=error"));
    assert!(answer.contains("exit code 127"));
}

#[test]
fn agent_loop_rich_content_precedes_status_summary_without_legacy_content_flag() {
    let mut route = free_route_result();
    route.semantic_kind = crate::OutputSemanticKind::None;
    route.response_shape = OutputResponseShape::Free;
    route.requires_content_evidence = false;
    route.delivery_required = false;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(4);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        "notes.txt\nnested/config.ini\n",
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "run_cmd",
        "fixture archive notes\n",
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_3",
        "db_basic",
        r#"{"extra":{"action":"schema_version","field_value":{"schema_version":7},"schema_version":7}}"#,
    ));
    loop_state.executed_step_results.push(err_step_result(
        "step_4",
        "fs_basic",
        "__RC_SKILL_ERROR__:{\"error_kind\":\"invalid_data\",\"error_text\":\"binary file is not utf8\"}",
    ));
    let delivery_messages = vec![
        "| item | value |\n| --- | --- |\n| archive members | notes.txt, nested/config.ini |\n| schema_version | 7 |".to_string(),
    ];

    assert!(
        successful_content_observation_should_precede_status_summary(
            Some(&agent_run_context),
            &loop_state,
        )
    );
    assert!(delivery_is_content_answer_candidate(
        Some(&agent_run_context),
        &loop_state,
        &delivery_messages,
    ));
}

#[test]
fn deterministic_missing_observed_target_answer_reports_missing_scalar_count_path() {
    let state = test_state();
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = OutputResponseShape::Scalar;
    route.semantic_kind = crate::OutputSemanticKind::None;
    route.selection.structured_field_selector = Some("count".to_string());
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "configs/config_copy".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "system_basic",
        r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":false,"path":"configs/config_copy"}],"include_missing":true}"#,
    ));

    let answer = deterministic_missing_observed_target_answer(
        &state,
        "查一下 configs/config_copy 下面有几个 toml 文件",
        &loop_state,
        Some(&agent_run_context),
    )
    .expect("missing path observation should produce a handled user answer");

    assert!(answer.contains("configs/config_copy"));
    assert!(answer.contains("exists=false"));
    assert!(answer.contains("final_answer_shape=scalar"));
    assert!(answer.contains("count_available=false"));
}

#[test]
fn deterministic_missing_observed_target_uses_generic_machine_shape() {
    let state = test_state();
    let mut route = free_route_result();
    route.requires_content_evidence = false;
    route.response_shape = OutputResponseShape::OneSentence;
    route.semantic_kind = crate::OutputSemanticKind::None;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "/home/guagua/rustclaw/document/nl_tool200/group_02/memo.txt".to_string();
    route.selection.structured_field_selector = Some("exists,path".to_string());
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":false,"path":"/home/guagua/rustclaw/document/nl_tool200/group_02/memo.txt","error":"not found"}],"include_missing":true}"#,
    ));

    let answer = deterministic_missing_observed_target_answer(
        &state,
        "检查 group_02 的 memo.txt 是否存在，只回答存在或不存在",
        &loop_state,
        Some(&agent_run_context),
    )
    .expect("missing existence observation should produce concise scalar answer");

    assert_eq!(
        answer,
        "schema_version=1\nreason_code=missing_observed_target\nexists=false\npath=`/home/guagua/rustclaw/document/nl_tool200/group_02/memo.txt`\nkind=missing\nfinal_answer_shape=summary_with_evidence"
    );
}

#[test]
fn deterministic_missing_observed_target_machine_payload_is_language_neutral() {
    let state = test_state();
    let mut route = free_route_result();
    route.requires_content_evidence = false;
    route.response_shape = OutputResponseShape::OneSentence;
    route.semantic_kind = crate::OutputSemanticKind::None;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "/tmp/rustclaw-missing-ja.txt".to_string();
    route.selection.structured_field_selector = Some("exists,path".to_string());
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        original_user_request: Some(
            "/tmp/rustclaw-missing-ja.txt が存在するか確認してください".to_string(),
        ),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":false,"path":"/tmp/rustclaw-missing-ja.txt","error":"not found"}],"include_missing":true}"#,
    ));

    let answer = deterministic_missing_observed_target_answer(
        &state,
        "/tmp/rustclaw-missing-ja.txt が存在するか確認してください",
        &loop_state,
        Some(&agent_run_context),
    )
    .expect("machine payload should not depend on natural-language template support");

    assert_eq!(
        answer,
        "schema_version=1\nreason_code=missing_observed_target\nexists=false\npath=`/tmp/rustclaw-missing-ja.txt`\nkind=missing\nfinal_answer_shape=summary_with_evidence"
    );
}

#[test]
fn deterministic_missing_observed_target_answer_skips_after_later_fallback_success() {
    let state = test_state();
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = OutputResponseShape::Scalar;
    route.selection.structured_field_selector = Some("path".to_string());
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "plan/missing.md".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "system_basic",
        r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":false,"path":"plan/missing.md"}]}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "fs_search",
        r#"{"action":"find_name","count":1,"patterns":["agent_intelligence"],"results":["plan/agent_intelligence_architecture_plan_20260511.md"],"root":"plan"}"#,
    ));

    assert!(deterministic_missing_observed_target_answer(
        &state,
        "读取缺失文件；如果不存在，就搜索 fallback 文件。",
        &loop_state,
        Some(&agent_run_context),
    )
    .is_none());

    let (answer, _) =
        direct_scalar_observed_answer(Some(&state), &loop_state, Some(&agent_run_context))
            .expect("fallback success should become scalar answer");
    assert_eq!(
        answer,
        "plan/agent_intelligence_architecture_plan_20260511.md"
    );
}

#[test]
fn deterministic_observed_execution_status_answer_uses_structured_run_cmd_stderr() {
    let state = test_state();
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    let err = format!(
        "__RC_SKILL_ERROR__:{}",
        serde_json::json!({
            "skill": "run_cmd",
            "error_kind": "nonzero_exit",
            "error_text": "Command failed with exit code 7",
            "platform": "linux",
            "extra": {
                "exit_code": 7,
                "stderr": "problem",
                "output_truncated": false
            }
        })
    );
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "run_cmd", "READY\n"));
    loop_state
        .executed_step_results
        .push(err_step_result("step_2", "run_cmd", &err));

    let answer = deterministic_observed_execution_status_answer(
        &state,
        "执行两个命令，告诉我退出码和错误输出。",
        &loop_state,
    )
    .expect("mixed observed results should produce deterministic answer");

    assert!(answer.contains("step.2.error_summary="), "answer: {answer}");
    assert!(
        answer.contains("step.2.error_kind=nonzero_exit"),
        "answer: {answer}"
    );
    assert!(answer.contains("step.2.exit_code=7"), "answer: {answer}");
    assert!(answer.contains("step.2.stderr=problem"), "answer: {answer}");
}

#[test]
fn deterministic_observed_execution_status_answer_attaches_before_llm_fallback() {
    let state = test_state();
    let task = claimed_task("task-deterministic-observed-status");
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "health_check",
        r#"{"ok":true}"#,
    ));
    loop_state.executed_step_results.push(err_step_result(
        "step_2",
        "run_cmd",
        "Command failed with exit code 127\nstderr:\nmissing command",
    ));
    let mut finalizer_summary = None;

    assert!(attach_deterministic_observed_execution_status_answer(
        &state,
        &task,
        "先检查健康，再执行缺失命令，然后总结哪一步成功了、哪一步失败了。",
        &mut loop_state,
        &mut finalizer_summary,
    ));

    assert_eq!(loop_state.delivery_messages.len(), 1);
    assert!(loop_state.delivery_messages[0].contains("step.1.skill=health_check"));
    assert!(loop_state.delivery_messages[0].contains("step.1.status=ok"));
    assert!(loop_state.delivery_messages[0].contains("step.2.skill=run_cmd"));
    assert!(loop_state.delivery_messages[0].contains("step.2.status=error"));
    let summary = finalizer_summary.expect("summary");
    assert_eq!(summary.completion_ok, Some(true));
    assert_eq!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    );
}

#[test]
fn observed_fallback_allowed_for_matrix_route_after_planned_synthesis() {
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = crate::OutputResponseShape::Strict;
    route.semantic_kind = crate::OutputSemanticKind::None;
    route.locator_kind = crate::OutputLocatorKind::Path;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","names":["a.schema.json"],"entries":[{"name":"a.schema.json","size_bytes":1}]}"#,
    ));
    push_raw_plan_text(
        &mut loop_state,
        r#"{"steps":[{"type":"synthesize_answer","evidence_refs":["last_output"]}]}"#,
    );

    assert!(should_try_observed_output_language_fallback(
        &loop_state,
        Some(&agent_run_context)
    ));
}

#[test]
fn content_answer_candidate_prevents_status_summary_replacement() {
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.response_shape = crate::OutputResponseShape::Strict;
    route.semantic_kind = crate::OutputSemanticKind::None;
    route.locator_kind = crate::OutputLocatorKind::Path;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","names":["intent_normalizer.schema.json"],"entries":[{"name":"intent_normalizer.schema.json","size_bytes":13124}]}"#,
    ));
    loop_state.executed_step_results.push(err_step_result(
        "step_2",
        "system_basic",
        "action `system_basic.validate_structured` was rejected by policy",
    ));
    let delivery_messages =
        vec!["intent_normalizer.schema.json 最大；这个目录保存 JSON Schema。".to_string()];

    assert!(delivery_is_content_answer_candidate(
        Some(&agent_run_context),
        &loop_state,
        &delivery_messages
    ));
}

#[test]
fn deterministic_observed_execution_status_answer_replaces_bad_synthesis() {
    let state = test_state();
    let task = claimed_task("task-deterministic-observed-status-replace");
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state
        .delivery_messages
        .push("步骤2未观察到执行结果，因此无法确认成功或失败。".to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "health_check",
        r#"{"ok":true}"#,
    ));
    loop_state.executed_step_results.push(err_step_result(
        "step_2",
        "run_cmd",
        "Command failed with exit code 127\nstderr:\nmissing command",
    ));
    let mut finalizer_summary = None;

    assert!(
        replace_delivery_with_deterministic_observed_execution_status_answer(
            &state,
            &task,
            "先检查健康，再执行缺失命令，然后总结哪一步成功了、哪一步失败了。",
            &mut loop_state,
            &mut finalizer_summary,
        )
    );

    assert_eq!(loop_state.delivery_messages.len(), 1);
    assert!(loop_state.delivery_messages[0].contains("step.2.skill=run_cmd"));
    assert!(loop_state.delivery_messages[0].contains("step.2.status=error"));
    assert!(!loop_state.delivery_messages[0].contains("无法确认成功或失败"));
    assert_eq!(
        finalizer_summary.and_then(|summary| summary.completion_ok),
        Some(true)
    );
}

#[test]
fn deterministic_observed_execution_status_keeps_recovered_content_answer() {
    let state = test_state();
    let task = claimed_task("task-deterministic-observed-status-recovered");
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    let answer =
        "目标文件不存在；候选路径：plan/llm_first_agent_convergence_plan_20260511_已完成.md"
            .to_string();
    loop_state.delivery_messages.push(answer.clone());
    loop_state.last_user_visible_respond = Some(answer.clone());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"exists":false}"#,
    ));
    loop_state.executed_step_results.push(err_step_result(
        "step_2",
        "read_file",
        "file not found: /home/guagua/rustclaw/plan/missing.md",
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_3",
        "fs_basic",
        r#"{"results":["plan/llm_first_agent_convergence_plan_20260511_已完成.md"]}"#,
    ));
    let mut finalizer_summary = None;

    assert!(
        !replace_delivery_with_deterministic_observed_execution_status_answer(
            &state,
            &task,
            "读取缺失文件；如果不存在就返回候选路径",
            &mut loop_state,
            &mut finalizer_summary,
        )
    );
    assert_eq!(loop_state.delivery_messages, vec![answer]);
    assert!(finalizer_summary.is_none());
}

#[test]
fn deterministic_observed_execution_status_keeps_planned_failed_step_answer() {
    let state = test_state();
    let task = claimed_task("task-deterministic-observed-status-keep-planned");
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "run two commands and report failed step".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![
                crate::PlanStep {
                    step_id: "step_1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "run_cmd".to_string(),
                    args: serde_json::json!({"command": "echo BEFORE_BREAK"}),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
                crate::PlanStep {
                    step_id: "step_2".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "run_cmd".to_string(),
                    args: serde_json::json!({
                        "command": "definitely_missing_command_rustclaw_user_ops_13579"
                    }),
                    depends_on: vec!["step_1".to_string()],
                    why: String::new(),
                },
                crate::PlanStep {
                    step_id: "step_3".to_string(),
                    action_type: "respond".to_string(),
                    skill: "respond".to_string(),
                    args: serde_json::json!({
                        "content": "第二步挂了，`definitely_missing_command_rustclaw_user_ops_13579` 命令不存在。"
                    }),
                    depends_on: vec!["step_2".to_string()],
                    why: String::new(),
                },
            ])),
            verify_result: None,
        });
    let planned =
        "第二步挂了，`definitely_missing_command_rustclaw_user_ops_13579` 命令不存在。".to_string();
    loop_state.delivery_messages.push(planned.clone());
    loop_state.last_user_visible_respond = Some(planned.clone());
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "run_cmd", "BEFORE_BREAK\n"));
    loop_state.executed_step_results.push(err_step_result(
        "step_2",
        "run_cmd",
        "Command failed with exit code 127\nstderr:\nmissing command",
    ));
    let mut finalizer_summary = None;

    assert!(!replace_delivery_with_deterministic_observed_execution_status_answer(
        &state,
        &task,
        "先执行 echo BEFORE_BREAK，再执行 definitely_missing_command_rustclaw_user_ops_13579，只告诉我哪一步挂了",
        &mut loop_state,
        &mut finalizer_summary,
    ));

    assert_eq!(loop_state.delivery_messages, vec![planned.clone()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(planned.as_str())
    );
    assert_eq!(
        finalizer_summary.and_then(|summary| summary.completion_ok),
        Some(true)
    );
}

#[test]
fn model_failure_delivery_is_not_replaced_by_generic_status() {
    let state = test_state();
    let task = claimed_task("task-deterministic-failed-step-only");
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "run two commands and identify only failed step".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![
                crate::PlanStep {
                    step_id: "step_1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "run_cmd".to_string(),
                    args: serde_json::json!({"command": "echo BEFORE_BREAK"}),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
                crate::PlanStep {
                    step_id: "step_2".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "run_cmd".to_string(),
                    args: serde_json::json!({
                        "command": "definitely_missing_command_rustclaw_user_ops_13579"
                    }),
                    depends_on: vec!["step_1".to_string()],
                    why: String::new(),
                },
            ])),
            verify_result: None,
        });
    loop_state.delivery_messages.push(
        "第 1 步 `run_cmd` 成功。第 2 步 `run_cmd` 失败：Command failed with exit code 127。"
            .to_string(),
    );
    loop_state.last_publishable_synthesis_output = loop_state.delivery_messages.last().cloned();
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "run_cmd", "BEFORE_BREAK\n"));
    loop_state.executed_step_results.push(err_step_result(
        "step_2",
        "run_cmd",
        "Command failed with exit code 127\nstderr:\nmissing command",
    ));
    let mut finalizer_summary = None;

    assert!(
        !replace_delivery_with_deterministic_observed_execution_status_answer(
            &state,
            &task,
            "先执行 echo BEFORE_BREAK，再执行 definitely_missing_command_rustclaw_user_ops_13579，只告诉我哪一步挂了",
            &mut loop_state,
            &mut finalizer_summary,
        )
    );

    assert_eq!(loop_state.delivery_messages.len(), 1);
    assert_eq!(
        loop_state.delivery_messages[0],
        "第 1 步 `run_cmd` 成功。第 2 步 `run_cmd` 失败：Command failed with exit code 127。"
    );
    assert!(finalizer_summary.is_none());
}

#[test]
fn structured_failure_request_prefers_final_respond_over_synthesis_stdout() {
    let task = claimed_task("task-failed-step-final-respond-over-synthesis");
    let mut route = free_route_result();
    route.response_shape = OutputResponseShape::Strict;
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::None;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(4);
    loop_state.has_tool_or_skill_output = true;
    loop_state.last_user_visible_respond = Some(
        "step_2: definitely_missing_command_rustclaw_render_ko_0605 failed with exit code 127"
            .to_string(),
    );
    loop_state.last_publishable_synthesis_output = Some("RC_RENDER_KO_OK".to_string());
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "run_cmd", "RC_RENDER_KO_OK\n"));
    loop_state.executed_step_results.push(err_step_result(
        "step_2",
        "run_cmd",
        "__RC_SKILL_ERROR__:{\"error_kind\":\"nonzero_exit\",\"error_text\":\"Command failed with exit code 127\",\"extra\":{\"command\":\"definitely_missing_command_rustclaw_render_ko_0605\",\"exit_category\":\"command_not_found\",\"exit_code\":127},\"skill\":\"run_cmd\"}",
    ));

    backfill_delivery_from_last_outputs(&task, &mut loop_state, Some(&ctx));

    assert_eq!(
        loop_state.delivery_messages,
        vec![
            "step_2: definitely_missing_command_rustclaw_render_ko_0605 failed with exit code 127"
                .to_string()
        ]
    );
}

#[test]
fn generic_execution_status_ignores_contract_gap_errors() {
    let state = test_state();
    let task = claimed_task("task-deterministic-failed-step-contract-gap");
    let mut loop_state = crate::agent_engine::LoopState::new(4);
    loop_state
        .delivery_messages
        .push("Step 1 failed. Step 3 failed: `exit0=$?`.".to_string());
    loop_state.last_publishable_synthesis_output = loop_state.delivery_messages.last().cloned();
    loop_state.executed_step_results.push(err_step_result(
        "step_1",
        "system_basic",
        r#"__RC_SKILL_ERROR__:{"error_kind":"contract_action_rejected","error_text":"action rejected by the current output contract","extra":{"failure_attribution":"contract_gap"},"skill":"system_basic"}"#,
    ));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_2", "run_cmd", "BEFORE_BREAK\n"));
    loop_state.executed_step_results.push(err_step_result(
        "step_3",
        "run_cmd",
        &crate::skills::structured_skill_error_from_parts(
            "run_cmd",
            "nonzero_exit",
            "Command failed with exit code 127",
            Some("linux"),
            Some(serde_json::json!({
                "command": "definitely_missing_command_rustclaw_67890",
                "exit_code": 127,
                "stderr": "bash: line 1: definitely_missing_command_rustclaw_67890: command not found\n",
                "stdout": serde_json::Value::Null,
            })),
        ),
    ));
    let mut finalizer_summary = None;

    let answer = deterministic_observed_execution_status_answer(
        &state,
        "Execute the command sequence and identify the failed command.",
        &loop_state,
    )
    .expect("generic execution status payload");
    assert!(
        !replace_delivery_with_deterministic_observed_execution_status_answer(
            &state,
            &task,
            "Execute the command sequence and identify the failed command.",
            &mut loop_state,
            &mut finalizer_summary,
        )
    );

    assert_eq!(loop_state.delivery_messages.len(), 1);
    assert!(
        !answer.contains("contract_action_rejected"),
        "contract-gap errors should remain excluded: {answer}"
    );
    assert_eq!(
        loop_state.delivery_messages[0],
        "Step 1 failed. Step 3 failed: `exit0=$?`."
    );
    assert!(finalizer_summary.is_none());
}

#[test]
fn deterministic_observed_execution_status_replaces_raw_success_output() {
    let state = test_state();
    let task = claimed_task("task-deterministic-observed-status-replace-raw");
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state
        .delivery_messages
        .push("THINK_BREAK_CN".to_string());
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "run_cmd", "THINK_BREAK_CN\n"));
    loop_state.executed_step_results.push(err_step_result(
        "step_2",
        "run_cmd",
        "Command failed with exit code 127\nstderr:\nbash: definitely_missing_command: command not found",
    ));
    let mut finalizer_summary = None;

    assert!(
        replace_delivery_with_deterministic_observed_execution_status_answer(
            &state,
            &task,
            "先执行第一个命令，再执行第二个命令，然后总结成功和失败分别是什么。",
            &mut loop_state,
            &mut finalizer_summary,
        )
    );

    assert_eq!(loop_state.delivery_messages.len(), 1);
    assert!(loop_state.delivery_messages[0].contains("step.1.skill=run_cmd"));
    assert!(loop_state.delivery_messages[0].contains("step.1.status=ok"));
    assert!(loop_state.delivery_messages[0].contains("step.2.skill=run_cmd"));
    assert!(loop_state.delivery_messages[0].contains("step.2.status=error"));
    assert!(!loop_state.delivery_messages[0].trim().eq("THINK_BREAK_CN"));
    assert_eq!(
        finalizer_summary.and_then(|summary| summary.completion_ok),
        Some(true)
    );
}

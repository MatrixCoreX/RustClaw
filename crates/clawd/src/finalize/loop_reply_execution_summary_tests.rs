use super::*;

#[test]
fn execution_output_not_found_projects_machine_json() {
    let step = err_step_result(
        "step_1",
        "read_file",
        "__RC_READ_FILE_NOT_FOUND__:/tmp/missing.txt",
    );

    let output = output_text_from_execution_result(&step).expect("machine output");
    let value: serde_json::Value = serde_json::from_str(&output).expect("json output");

    assert_eq!(
        value
            .pointer("/message_key")
            .and_then(serde_json::Value::as_str),
        Some("clawd.msg.execution.step_observation")
    );
    assert_eq!(
        value
            .pointer("/reason_code")
            .and_then(serde_json::Value::as_str),
        Some("read_file_not_found")
    );
    assert_eq!(
        value
            .pointer("/error_kind")
            .and_then(serde_json::Value::as_str),
        Some("not_found")
    );
    assert_eq!(
        value.pointer("/path").and_then(serde_json::Value::as_str),
        Some("/tmp/missing.txt")
    );
    assert!(!output.contains("file not found"));
}

#[test]
fn execution_output_structured_error_projects_machine_json_without_error_text() {
    let step = err_step_result(
        "step_1",
        "run_cmd",
        r#"__RC_SKILL_ERROR__:{"skill":"run_cmd","error_kind":"nonzero_exit","error_text":"Command failed with exit code 127","extra":{"command":"missing-bin","exit_code":127,"stderr":"missing-bin: command not found"}}"#,
    );

    let output = output_text_from_execution_result(&step).expect("machine output");
    let value: serde_json::Value = serde_json::from_str(&output).expect("json output");

    assert_eq!(
        value
            .pointer("/reason_code")
            .and_then(serde_json::Value::as_str),
        Some("structured_skill_error")
    );
    assert_eq!(
        value
            .pointer("/error_kind")
            .and_then(serde_json::Value::as_str),
        Some("nonzero_exit")
    );
    assert_eq!(
        value
            .pointer("/extra/exit_code")
            .and_then(serde_json::Value::as_i64),
        Some(127)
    );
    assert!(value.pointer("/error_text").is_none());
    assert!(!output.contains("Command failed with exit code"));
}

#[test]
fn execution_output_json_strips_text_and_error_text_fields() {
    let step = ok_step_result(
        "step_1",
        "service_control",
        r#"{"text":"visible prose should not be protocol","error_text":"error prose should not be protocol","extra":{"action":"status","service_name":"clawd","post_state":"running"}}"#,
    );

    let output = output_text_from_execution_result(&step).expect("machine output");
    let value: serde_json::Value = serde_json::from_str(&output).expect("json output");

    assert!(value.pointer("/text").is_none());
    assert!(value.pointer("/error_text").is_none());
    assert_eq!(
        value
            .pointer("/extra/action")
            .and_then(serde_json::Value::as_str),
        Some("status")
    );
    assert_eq!(
        value
            .pointer("/reason_code")
            .and_then(serde_json::Value::as_str),
        Some("json_observation")
    );
}

#[test]
fn execution_summary_suppressed_for_grounded_content_answer() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","excerpt":"1|{\n2|  \"type\": \"object\",\n3|  \"additionalProperties\": false\n4|}","path":"prompts/schemas/agent_loop_decision_envelope.schema.json"}"#,
    ));
    loop_state.delivery_messages.push(
        "`additionalProperties: false` makes future schema extension more brittle.".to_string(),
    );
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::ConfigRiskAssessment;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "prompts/schemas/agent_loop_decision_envelope.schema.json".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..crate::agent_engine::AgentRunContext::default()
    };

    assert!(delivery_contract_suppresses_execution_summary(
        &loop_state,
        Some(&ctx),
        &loop_state.delivery_messages
    ));
    assert!(build_execution_summary_messages(
        &loop_state,
        Some(&ctx),
        Some("Check the schema risks briefly.")
    )
    .is_empty());
}

#[test]
fn execution_summary_is_not_attached_before_final_delivery() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "list recent logs".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![crate::PlanStep {
                step_id: "step_1".to_string(),
                action_type: "call_tool".to_string(),
                skill: "run_cmd".to_string(),
                args: serde_json::json!({"command": "ls -t logs | head -2"}),
                depends_on: Vec::new(),
                why: String::new(),
            }])),
            verify_result: None,
        });
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        "model_io.log\nact_plan.log\n",
    ));
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(free_route_result()),
        ..Default::default()
    };
    let mut delivery = vec!["这更像运行日志。".to_string()];

    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

    assert_eq!(delivery.len(), 1);
    assert_eq!(
        delivery.last().map(String::as_str),
        Some("这更像运行日志。")
    );
    assert!(delivery
        .iter()
        .all(|message| !crate::finalize::is_execution_summary_message(message)));
    assert!(crate::task_journal::delivery_payload_consistent(
        "这更像运行日志。",
        &delivery
    ));
    assert_eq!(
        final_answer_text_from_delivery(&delivery),
        "这更像运行日志。"
    );
}

#[test]
fn contract_matrix_delivery_suppresses_hardcoded_execution_summary() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "list archive members".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![crate::PlanStep {
                step_id: "step_1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "archive_basic".to_string(),
                args: serde_json::json!({"action": "list"}),
                depends_on: Vec::new(),
                why: String::new(),
            }])),
            verify_result: None,
        });
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "archive_basic",
        "notes.txt\nnested/config.ini\n",
    ));
    let mut route = free_route_result();
    route.response_shape = crate::OutputResponseShape::Strict;
    route.semantic_kind = crate::OutputSemanticKind::ArchiveList;
    route.requires_content_evidence = true;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut delivery = vec![
        "**执行过程**\n1. 调用技能 `archive_basic`".to_string(),
        "notes.txt\nnested/config.ini".to_string(),
    ];

    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

    assert_eq!(delivery, vec!["notes.txt\nnested/config.ini".to_string()]);
}

#[test]
fn evidence_contract_delivery_suppresses_execution_summary_for_name_list_answer() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","names":["README.txt"],"names_only":true}"#,
    ));
    let mut route = free_route_result();
    route.response_shape = crate::OutputResponseShape::Strict;
    route.semantic_kind = crate::OutputSemanticKind::FileNames;
    route.requires_content_evidence = true;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut delivery = vec![
        "**Execution**\n1. Called tool `fs_basic`".to_string(),
        "README.txt".to_string(),
    ];

    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

    assert_eq!(delivery, vec!["README.txt".to_string()]);
}

#[test]
fn evidence_contract_delivery_suppresses_execution_summary_for_status_answer() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "http_basic",
        r#"status=200 {"ok":true}"#,
    ));
    let mut route = free_route_result();
    route.response_shape = crate::OutputResponseShape::Free;
    route.semantic_kind = crate::OutputSemanticKind::ServiceStatus;
    route.requires_content_evidence = true;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut delivery = vec![
        "**Execution**\n1. Called skill `http_basic`".to_string(),
        "The health endpoint is reachable with HTTP 200.".to_string(),
    ];

    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

    assert_eq!(
        delivery,
        vec!["The health endpoint is reachable with HTTP 200.".to_string()]
    );
}

#[test]
fn execution_summary_is_not_attached_for_japanese_request() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "logs ディレクトリのファイル名を3つだけ一覧して。".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![crate::PlanStep {
                step_id: "step_1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "system_basic".to_string(),
                args: serde_json::json!({
                    "action": "inventory_dir",
                    "path": "/tmp/logs",
                    "names_only": true,
                    "max_entries": 3
                }),
                depends_on: Vec::new(),
                why: String::new(),
            }])),
            verify_result: None,
        });
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "system_basic",
        "act_plan.log\nclawd.log\nclawd.run.log\n",
    ));
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(free_route_result()),
        original_user_request: Some("logs ディレクトリのファイル名を3つだけ一覧して。".to_string()),
        ..Default::default()
    };
    let mut delivery = vec!["act_plan.log\nclawd.log\nclawd.run.log".to_string()];

    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

    assert_eq!(delivery, vec!["act_plan.log\nclawd.log\nclawd.run.log"]);
    assert!(delivery
        .iter()
        .all(|message| !crate::finalize::is_execution_summary_message(message)));
}

#[test]
fn execution_summary_suppressed_for_scalar_value_contract() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "extract package name".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![crate::PlanStep {
                step_id: "step_1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "system_basic".to_string(),
                args: serde_json::json!({
                    "action": "extract_field",
                    "path": "/tmp/package.json",
                    "field_path": "name"
                }),
                depends_on: Vec::new(),
                why: String::new(),
            }])),
            verify_result: None,
        });
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "system_basic",
        r#"{"action":"extract_field","field_path":"name","value_text":"rustclaw-nl-fixture"}"#,
    ));
    let mut route = scalar_route_result();
    route.response_shape = crate::OutputResponseShape::Strict;
    route.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut delivery = vec!["rustclaw-nl-fixture".to_string()];

    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

    assert_eq!(delivery, vec!["rustclaw-nl-fixture"]);
    assert!(build_execution_summary_message(&loop_state, Some(&ctx), None).is_none());
}

#[test]
fn execution_summary_drops_existing_summary_for_scalar_delivery_contract() {
    let route = scalar_route_result();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "config_basic",
        r#"{"action":"extract_field","exists":true,"field_path":"description","value_text":"Local fixture package for RustClaw NL regression tests"}"#,
    ));
    let mut delivery = vec![
        "**実行過程**\n1. ツール `config_basic`を呼び出しました".to_string(),
        "Local fixture package for RustClaw NL regression tests".to_string(),
    ];

    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

    assert_eq!(
        delivery,
        vec!["Local fixture package for RustClaw NL regression tests"]
    );
    assert!(!delivery
        .iter()
        .any(|message| crate::finalize::is_execution_summary_message(message)));
}

#[test]
fn execution_summary_drops_existing_summary_when_structured_keys_delivery_matches_scalar_observation(
) {
    let mut route = free_route_result();
    route.response_shape = crate::OutputResponseShape::Strict;
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::StructuredKeys;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "config_basic",
        r#"{"action":"structured_keys","path":"package.json","keys":["scripts.lint"]}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "config_basic",
        r#"{"action":"extract_field","exists":true,"field_path":"scripts.lint","value":"echo lint","value_text":"echo lint","value_type":"string"}"#,
    ));
    let mut delivery = vec![
        "**Execution**\n1. Called tool `config_basic` with action `structured_keys`.".to_string(),
        "**Execution**\n2. Called tool `config_basic` with action `extract_field`.".to_string(),
        "echo lint".to_string(),
    ];

    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

    assert_eq!(delivery, vec!["echo lint"]);
    assert!(!delivery
        .iter()
        .any(|message| crate::finalize::is_execution_summary_message(message)));
}

#[test]
fn execution_summary_drops_existing_summary_for_config_guard_delivery() {
    let mut route = free_route_result();
    route.response_shape = crate::OutputResponseShape::Free;
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::ConfigRiskAssessment;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "config_edit",
        r#"{"action":"guard_config","format":"toml","path":"/home/guagua/rustclaw/configs/config.toml","resolved_path":"/home/guagua/rustclaw/configs/config.toml","risk_count":3,"risks":["tools.allow_sudo=true","tools.allow_path_outside_workspace=true","telegram.sendfile.full_access=true"]}"#,
    ));
    let answer = "Found 3 config risk(s) in `/home/guagua/rustclaw/configs/config.toml`: tools.allow_sudo=true; tools.allow_path_outside_workspace=true; telegram.sendfile.full_access=true.";
    let mut delivery = vec![
        "**実行過程**\n1. スキル `config_edit`（action=guard_config）を呼び出しました".to_string(),
        answer.to_string(),
    ];

    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

    assert_eq!(delivery, vec![answer]);
    assert!(!delivery
        .iter()
        .any(|message| crate::finalize::is_execution_summary_message(message)));
}

#[test]
fn execution_summary_drops_existing_summary_for_transform_result_delivery() {
    let mut route = free_route_result();
    route.response_shape = crate::OutputResponseShape::Strict;
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::None;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "transform",
        r#"{"status":"ok","formatted":null,"result":[{"name":"beta"},{"name":"alpha"}]}"#,
    ));
    let answer = r#"[{"name":"beta"},{"name":"alpha"}]"#;
    let mut delivery = vec![
        "**Execution**\n1. Called skill `transform` (action=transform_data)".to_string(),
        answer.to_string(),
    ];

    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

    assert_eq!(delivery, vec![answer]);
    assert!(!delivery
        .iter()
        .any(|message| crate::finalize::is_execution_summary_message(message)));
}

#[test]
fn execution_summary_drops_existing_summary_for_strict_synthesized_delivery() {
    let mut route = free_route_result();
    route.response_shape = crate::OutputResponseShape::Strict;
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","names":["README.md","configs"],"counts":{"total":2}}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "synthesize_answer",
        "README.md is documentation; configs contains settings.",
    ));
    let answer = "README.md is documentation; configs contains settings.";
    let mut delivery = vec![
        "**Execution**\n1. Called tool `fs_basic` (action=inventory_dir)".to_string(),
        answer.to_string(),
    ];

    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

    assert_eq!(delivery, vec![answer]);
    assert!(!delivery
        .iter()
        .any(|message| crate::finalize::is_execution_summary_message(message)));
}

#[test]
fn execution_summary_drops_existing_summary_for_synthesized_content_delivery() {
    let mut route = free_route_result();
    route.response_shape = crate::OutputResponseShape::Free;
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::None;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "config_basic",
        r#"{"action":"extract_field","exists":true,"field_path":"scripts","value":{"build":"echo build","dev":"echo dev","lint":"echo lint"},"value_text":"{\"build\":\"echo build\",\"dev\":\"echo dev\",\"lint\":\"echo lint\"}","value_type":"object"}"#,
    ));
    let mut delivery = vec![
        "**执行过程**\n1. 调用工具 `config_basic`".to_string(),
        "该 scripts 字段定义了 build、dev、lint 三个脚本，均为 echo 占位命令。".to_string(),
    ];

    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

    assert_eq!(
        delivery,
        vec!["该 scripts 字段定义了 build、dev、lint 三个脚本，均为 echo 占位命令。"]
    );
    assert!(!delivery
        .iter()
        .any(|message| crate::finalize::is_execution_summary_message(message)));
}

#[test]
fn execution_summary_suppressed_for_multi_structured_scalar_synthesis() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "config_basic",
        r#"{"action":"extract_field","exists":true,"field_path":"name","value_text":"rustclaw-nl-fixture"}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "config_basic",
        r#"{"action":"extract_field","exists":true,"field_path":"package.name","value_text":"clawd"}"#,
    ));
    loop_state.last_publishable_synthesis_output = Some("rustclaw-nl-fixture != clawd".to_string());
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    assert!(build_execution_summary_message(&loop_state, Some(&ctx), None).is_none());
}

#[test]
fn execution_summary_suppressed_for_scalar_content_synthesis() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r##"{"action":"read_text_range","path":"/tmp/release_checklist.md","content":"# Release Checklist\n\n1. Verify config."}"##,
    ));
    loop_state.last_publishable_synthesis_output = Some("Release Checklist".to_string());
    let mut route = scalar_route_result();
    route.response_shape = crate::OutputResponseShape::Scalar;
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    assert!(build_execution_summary_message(&loop_state, Some(&ctx), None).is_none());
}

#[test]
fn execution_summary_is_not_attached_for_multiple_execution_steps() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "tell joke and print pwd".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![
                crate::PlanStep {
                    step_id: "step_1".to_string(),
                    action_type: "call_tool".to_string(),
                    skill: "run_cmd".to_string(),
                    args: serde_json::json!({"command": "pwd"}),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
                crate::PlanStep {
                    step_id: "step_2".to_string(),
                    action_type: "call_tool".to_string(),
                    skill: "run_cmd".to_string(),
                    args: serde_json::json!({"command": "date"}),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
            ])),
            verify_result: None,
        });
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        "/home/guagua/rustclaw\n",
    ));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_2", "run_cmd", "Sun May 3\n"));
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(free_route_result()),
        ..Default::default()
    };
    let mut delivery = vec!["为什么程序员喜欢黑夜？因为 bug 比较容易显现。".to_string()];

    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

    assert_eq!(delivery.len(), 1);
    assert_eq!(
        delivery.last().map(String::as_str),
        Some("为什么程序员喜欢黑夜？因为 bug 比较容易显现。")
    );
    assert!(delivery
        .iter()
        .all(|message| !crate::finalize::is_execution_summary_message(message)));
}

#[test]
fn execution_summary_message_builder_returns_none_for_english_requests() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "list recent logs".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![crate::PlanStep {
                step_id: "step_1".to_string(),
                action_type: "call_tool".to_string(),
                skill: "run_cmd".to_string(),
                args: serde_json::json!({"command": "ls -t logs | head -2"}),
                depends_on: Vec::new(),
                why: String::new(),
            }])),
            verify_result: None,
        });
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        "model_io.log\nact_plan.log\n",
    ));
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(free_route_result()),
        ..Default::default()
    };

    assert!(build_execution_summary_message(
        &loop_state,
        Some(&ctx),
        Some("List the two most recently modified files in logs, then tell me what they are."),
    )
    .is_none());
}

#[test]
fn execution_summary_builder_stays_disabled_for_shifted_rounds() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "pack archive".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![
                crate::PlanStep {
                    step_id: "step_1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "archive_basic".to_string(),
                    args: serde_json::json!({"action": "pack"}),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
                crate::PlanStep {
                    step_id: "step_2".to_string(),
                    action_type: "respond".to_string(),
                    skill: "respond".to_string(),
                    args: serde_json::json!({}),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
            ])),
            verify_result: None,
        });
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 2,
            goal: "verify archive".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![
                crate::PlanStep {
                    step_id: "step_1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "system_basic".to_string(),
                    args: serde_json::json!({"action": "path_batch_facts"}),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
                crate::PlanStep {
                    step_id: "step_2".to_string(),
                    action_type: "respond".to_string(),
                    skill: "respond".to_string(),
                    args: serde_json::json!({}),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
            ])),
            verify_result: None,
        });
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "archive_basic", "exit=0\n"));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "system_basic",
        r#"{"action":"path_batch_facts","count":1}"#,
    ));
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(free_route_result()),
        ..Default::default()
    };

    assert!(build_execution_summary_message(
        &loop_state,
        Some(&ctx),
        Some("Zip scripts/skill_calls into tmp/nl_archive_case_en.zip, then tell me briefly whether it succeeded."),
    )
    .is_none());
}

#[test]
fn execution_summary_builder_stays_disabled_when_global_step_ids_shift() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "read old config field".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![crate::PlanStep {
                step_id: "step_1".to_string(),
                action_type: "call_tool".to_string(),
                skill: "config_basic".to_string(),
                args: serde_json::json!({"action": "read_field"}),
                depends_on: Vec::new(),
                why: String::new(),
            }])),
            verify_result: None,
        });
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 2,
            goal: "edit config field".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![
                crate::PlanStep {
                    step_id: "step_1".to_string(),
                    action_type: "call_tool".to_string(),
                    skill: "config_edit".to_string(),
                    args: serde_json::json!({"action": "plan_config_change"}),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
                crate::PlanStep {
                    step_id: "step_2".to_string(),
                    action_type: "call_tool".to_string(),
                    skill: "config_edit".to_string(),
                    args: serde_json::json!({"action": "apply_config_change"}),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
                crate::PlanStep {
                    step_id: "step_3".to_string(),
                    action_type: "call_tool".to_string(),
                    skill: "config_edit".to_string(),
                    args: serde_json::json!({"action": "validate_config"}),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
            ])),
            verify_result: None,
        });
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "config_edit",
        r#"{"action":"plan_config_change"}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_3",
        "config_edit",
        r#"{"action":"apply_config_change"}"#,
    ));

    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(free_route_result()),
        ..Default::default()
    };
    assert!(
        build_execution_summary_message(&loop_state, Some(&ctx), Some("把配置项打开")).is_none()
    );
}

#[test]
fn virtual_tool_execution_summary_builder_stays_disabled_without_plan_step() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","count":5}"#,
    ));
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(free_route_result()),
        ..Default::default()
    };

    assert!(build_execution_summary_message(
        &loop_state,
        Some(&ctx),
        Some("列出当前目录最近修改的文件"),
    )
    .is_none());
}

#[test]
fn virtual_tool_execution_summary_builder_stays_disabled_when_plan_used_call_skill() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "compare file sizes".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![crate::PlanStep {
                step_id: "step_1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "fs_basic".to_string(),
                args: serde_json::json!({"action": "stat_paths"}),
                depends_on: Vec::new(),
                why: String::new(),
            }])),
            verify_result: None,
        });
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"path_batch_facts","count":2}"#,
    ));
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(free_route_result()),
        ..Default::default()
    };

    assert!(
        build_execution_summary_message(&loop_state, Some(&ctx), Some("Compare file sizes."))
            .is_none()
    );
}

#[test]
fn observed_synthesis_unavailable_fails_loud_without_execution_summary() {
    let state = test_state();
    let task = claimed_task("task-observed-llm-unavailable");
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        "Cargo.toml\nREADME.md\n",
    ));
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(free_route_result()),
        ..Default::default()
    };

    let reply = observed_synthesis_unavailable_reply(
        &state,
        &task,
        "列一下当前目录，然后总结一下",
        &loop_state,
        Some(&ctx),
        "No available LLM provider configured",
    );

    assert!(reply.should_fail_task);
    assert!(!reply.text.trim().is_empty());
    assert_eq!(reply.messages.last(), Some(&reply.text));
    assert_eq!(reply.messages.len(), 1);
    assert!(reply
        .messages
        .iter()
        .all(|message| !crate::finalize::is_execution_summary_message(message)));
    assert_eq!(
        reply
            .task_journal
            .as_ref()
            .and_then(|journal| journal.final_status),
        Some(crate::task_journal::TaskJournalFinalStatus::Failure)
    );
}

#[test]
fn execution_summary_is_not_attached_for_exact_observed_passthrough_delivery() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "print pwd".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![crate::PlanStep {
                step_id: "step_1".to_string(),
                action_type: "call_tool".to_string(),
                skill: "run_cmd".to_string(),
                args: serde_json::json!({"command": "pwd"}),
                depends_on: Vec::new(),
                why: String::new(),
            }])),
            verify_result: None,
        });
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        "/home/guagua/rustclaw\n",
    ));
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(free_route_result()),
        ..Default::default()
    };
    let mut delivery = vec!["/home/guagua/rustclaw".to_string()];

    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

    assert_eq!(delivery.len(), 1);
    assert_eq!(
        delivery.last().map(String::as_str),
        Some("/home/guagua/rustclaw")
    );
    assert!(delivery
        .iter()
        .all(|message| !crate::finalize::is_execution_summary_message(message)));
}

#[test]
fn execution_summary_skips_for_raw_command_output_route() {
    let mut route = free_route_result();
    route.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        "/home/guagua/rustclaw\n",
    ));

    assert!(build_execution_summary_message(&loop_state, Some(&ctx), None).is_none());
}

#[test]
fn execution_summary_suppressed_for_strict_content_excerpt_contract() {
    let mut route = free_route_result();
    route.response_shape = crate::OutputResponseShape::Strict;
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "read tail".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![crate::PlanStep {
                step_id: "step_1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "system_basic".to_string(),
                args: serde_json::json!({
                    "action": "read_range",
                    "path": "/tmp/model_io.log",
                    "mode": "tail",
                    "n": 10
                }),
                depends_on: Vec::new(),
                why: String::new(),
            }])),
            verify_result: None,
        });
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "system_basic",
        r#"{"action":"read_range","excerpt":"1|alpha\n2|beta","path":"/tmp/model_io.log"}"#,
    ));

    assert!(build_execution_summary_message(&loop_state, Some(&ctx), None).is_none());
}

#[test]
fn execution_summary_suppressed_for_generic_path_content_contract() {
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "logs/clawd.log".to_string();
    route.semantic_kind = crate::OutputSemanticKind::None;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","mode":"tail","requested_n":2,"excerpt":"1|alpha\n2|beta","path":"logs/clawd.log"}"#,
    ));
    let mut delivery = vec![
        "**Execution**\n1. Called tool `fs_basic`".to_string(),
        "alpha\nbeta".to_string(),
    ];

    assert!(build_execution_summary_message(&loop_state, Some(&ctx), None).is_none());
    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

    assert_eq!(delivery, vec!["alpha\nbeta".to_string()]);
}

#[test]
fn execution_summary_sanitizes_log_excerpt_secrets_and_ansi() {
    let mut route = free_route_result();
    route.response_shape = crate::OutputResponseShape::Strict;
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "system_basic",
        r#"{"action":"read_range","excerpt":"1|\u001b[32mconnected\u001b[0m to wss://host/ws?device_id=123&access_key=abc123&service_id=7&ticket=deadbeef","path":"/tmp/feishud.log"}"#,
    ));

    assert!(build_execution_summary_message(&loop_state, Some(&ctx), None).is_none());
}

#[test]
fn execution_summary_suppressed_for_exact_file_names_contract() {
    let mut route = free_route_result();
    route.locator_hint = "document".to_string();
    route.semantic_kind = crate::OutputSemanticKind::FileNames;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "list_dir",
        "alpha.md\nbeta.md\n",
    ));
    let mut delivery = vec!["alpha.md\nbeta.md".to_string()];

    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

    assert_eq!(delivery, vec!["alpha.md\nbeta.md"]);
    assert!(build_execution_summary_message(&loop_state, Some(&ctx), None).is_none());
}

#[test]
fn execution_summary_skips_for_exact_sentence_count_contract() {
    let mut route = free_route_result();
    route.response_shape = crate::OutputResponseShape::Strict;
    route.requires_content_evidence = true;
    route.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.exact_sentence_count = Some(3);
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "doc_parse",
        "RustClaw is a local Rust agent runtime centered on clawd.",
    ));
    let mut delivery = vec![
        "RustClaw 是一个本地 Rust agent 运行时。它以 clawd 为核心。它面向多渠道任务执行。"
            .to_string(),
    ];

    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

    assert_eq!(delivery.len(), 1);
    assert!(!crate::finalize::is_execution_summary_message(&delivery[0]));
    assert!(build_execution_summary_message(&loop_state, Some(&ctx), None).is_none());
}

#[test]
fn execution_summary_skips_for_scalar_count_contract() {
    let mut route = scalar_route_result();
    route.semantic_kind = crate::OutputSemanticKind::ScalarCount;
    route.response_shape = crate::OutputResponseShape::Scalar;
    route.requires_content_evidence = true;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"count_inventory","counts":{"total":64}}"#,
    ));
    let mut delivery = vec!["64".to_string()];

    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

    assert_eq!(delivery, vec!["64"]);
    assert!(build_execution_summary_message(&loop_state, Some(&ctx), None).is_none());
}

#[test]
fn execution_summary_skips_for_scalar_count_inventory_observation() {
    let mut route = scalar_route_result();
    route.semantic_kind = crate::OutputSemanticKind::None;
    route.response_shape = crate::OutputResponseShape::Scalar;
    route.requires_content_evidence = true;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"count_inventory","counts":{"total":64}}"#,
    ));
    let mut delivery = vec!["64".to_string()];

    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

    assert_eq!(delivery, vec!["64"]);
    assert!(build_execution_summary_message(&loop_state, Some(&ctx), None).is_none());
}

#[test]
fn execution_summary_skips_for_strict_json_container_delivery() {
    let mut route = free_route_result();
    route.response_shape = crate::OutputResponseShape::Strict;
    route.requires_content_evidence = true;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "db_basic",
        r#"{"columns":["id","name"],"rows":[{"id":1,"name":"Alice"}]}"#,
    ));
    let mut delivery = vec![
        "**执行过程**\n1. 调用技能 `db_basic`".to_string(),
        r#"[{"id":1,"name":"Alice"}]"#.to_string(),
    ];

    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

    assert_eq!(delivery, vec![r#"[{"id":1,"name":"Alice"}]"#.to_string()]);
}

#[test]
fn execution_summary_suppressed_for_file_names_contract_even_with_original_user_request() {
    let mut route = free_route_result();
    route.semantic_kind = crate::OutputSemanticKind::FileNames;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        original_user_request: Some("先列出 logs 目录下前 5 个文件名".to_string()),
        user_request: Some("List the first five filenames under logs.".to_string()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "list_dir",
        "act_plan.log\nclawd.log\n",
    ));

    assert!(build_execution_summary_message(
        &loop_state,
        Some(&ctx),
        Some("List the first five filenames under logs."),
    )
    .is_none());
}

#[test]
fn execution_summary_is_not_attached_for_failed_file_token_delivery() {
    let mut route = free_route_result();
    route.response_shape = crate::OutputResponseShape::FileToken;
    route.delivery_required = true;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "send file".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![crate::PlanStep {
                step_id: "step_1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "read_file".to_string(),
                args: serde_json::json!({"path": "/tmp/missing.txt"}),
                depends_on: Vec::new(),
                why: String::new(),
            }])),
            verify_result: None,
        });
    loop_state.executed_step_results.push(err_step_result(
        "step_1",
        "read_file",
        "__RC_READ_FILE_NOT_FOUND__:/tmp/missing.txt",
    ));
    let mut delivery = vec!["File not found at the provided path.".to_string()];

    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

    assert_eq!(delivery.len(), 1);
    assert_eq!(
        delivery.last().map(String::as_str),
        Some("File not found at the provided path.")
    );
    assert!(delivery
        .iter()
        .all(|message| !crate::finalize::is_execution_summary_message(message)));
}

#[test]
fn execution_summary_suppressed_for_successful_file_token_delivery() {
    let mut route = free_route_result();
    route.response_shape = crate::OutputResponseShape::FileToken;
    route.delivery_required = true;
    route.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: "send file".to_string(),
            execution_recipe_summary: None,
            plan_result: Some(plan_result_with_steps(vec![crate::PlanStep {
                step_id: "step_1".to_string(),
                action_type: "call_tool".to_string(),
                skill: "fs_basic".to_string(),
                args: serde_json::json!({
                    "action": "path_batch_facts",
                    "path": "/tmp/report.txt"
                }),
                depends_on: Vec::new(),
                why: String::new(),
            }])),
            verify_result: None,
        });
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":true,"path":"/tmp/report.txt","fact":{"kind":"file","resolved_path":"/tmp/report.txt"}}]}"#,
    ));
    let mut delivery = vec![
        "**执行过程**\n1. 调用工具 `fs_basic`".to_string(),
        "FILE:/tmp/report.txt".to_string(),
    ];

    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

    assert_eq!(delivery, vec!["FILE:/tmp/report.txt".to_string()]);
}

#[test]
fn execution_summary_suppressed_for_existence_with_path_contract() {
    let mut route = free_route_result();
    route.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
    route.locator_hint = "rustclaw.service".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_search",
        r#"{"action":"find_name","count":1,"results":["rustclaw.service"]}"#,
    ));
    let mut delivery = vec!["有，路径：rustclaw.service".to_string()];

    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

    assert_eq!(delivery, vec!["有，路径：rustclaw.service"]);
    assert!(build_execution_summary_message(&loop_state, Some(&ctx), None).is_none());
}

#[test]
fn execution_summary_includes_direct_fs_search_structured_observation() {
    let route = free_route_result();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_search",
        r#"{"action":"find_name","count":1,"results":["rustclaw.service"],"root":""}"#,
    ));
    let mut delivery = vec!["有，路径：rustclaw.service".to_string()];

    attach_execution_summary_to_delivery(&loop_state, Some(&ctx), None, &mut delivery);

    assert_eq!(delivery.len(), 1);
    assert_eq!(
        delivery.last().map(String::as_str),
        Some("有，路径：rustclaw.service")
    );
    assert!(delivery
        .iter()
        .all(|message| !crate::finalize::is_execution_summary_message(message)));
}

#[test]
fn execution_summary_suppressed_for_scalar_contract_without_reading_user_text() {
    let mut route = free_route_result();
    route.response_shape = OutputResponseShape::Scalar;
    route.semantic_kind = crate::OutputSemanticKind::HiddenEntriesCheck;
    route.locator_hint = ".".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "list_dir",
        ".git\n.gitignore\n",
    ));
    let mut delivery = vec!["有。示例：.git, .gitignore".to_string()];

    attach_execution_summary_to_delivery(
        &loop_state,
        Some(&ctx),
        Some("plain runtime text that is intentionally ignored"),
        &mut delivery,
    );

    assert_eq!(delivery, vec!["有。示例：.git, .gitignore"]);
    assert!(build_execution_summary_message(
        &loop_state,
        Some(&ctx),
        Some("plain runtime text that is intentionally ignored"),
    )
    .is_none());
}

#[test]
fn execution_summary_builder_stays_disabled_for_long_outputs() {
    let ctx = crate::agent_engine::AgentRunContext {
        output_contract: Some(free_route_result()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    let long_output = format!("{}END", "x".repeat(1000));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "system_basic", &long_output));

    assert!(build_execution_summary_message(&loop_state, Some(&ctx), None).is_none());
}

#[test]
fn execution_summary_builder_stays_disabled_for_recoverable_crypto_account_error() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_recoverable_failure_context = true;
    let err = r#"__RC_CRYPTO_ACCOUNT_ACCESS_ERROR__:{"exchange":"binance","detail":"binance error status=401: {\"code\":-2015,\"msg\":\"Invalid API-key, IP, or permissions for action.\"}"}"#;
    loop_state
        .executed_step_results
        .push(err_step_result("step_1", "crypto", err));

    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(free_route_result()),
        ..Default::default()
    };
    let summaries =
        build_execution_summary_messages(&loop_state, Some(&agent_run_context), Some("查一下持仓"));

    assert!(summaries.is_empty());
}

#[test]
fn content_evidence_failure_suppresses_execution_summary_for_missing_target() {
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(err_step_result(
        "step_1",
        "system_basic",
        "__RC_READ_FILE_NOT_FOUND__:plan/does_not_exist_builtin_tool_case.toml",
    ));

    assert!(super::super::content_evidence_failure_suppresses_execution_summary(&loop_state));
}

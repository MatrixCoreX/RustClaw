use super::*;

#[test]
fn terminal_synthesis_placeholder_respond_uses_last_output() {
    let actions = vec![
        AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: json!({ "path": "README.md" }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["step_1".to_string()],
        },
        AgentAction::Respond {
            content: "{{synthesized}}".to_string(),
        },
    ];

    let out = rewrite_terminal_synthesis_placeholder_respond(actions);
    assert_eq!(out.len(), 3);
    assert!(matches!(
        &out[2],
        AgentAction::Respond { content } if content == "{{last_output}}"
    ));
}

#[test]
fn injection_no_op_when_respond_content_is_concrete() {
    let actions = vec![
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({ "command": "whoami" }),
        },
        AgentAction::Respond {
            content: "guagua".to_string(),
        },
    ];
    let before = actions_as_json(&actions);
    let out = inject_synthesize_answer_for_bare_placeholder_respond(actions, "x");
    assert_eq!(actions_as_json(&out), before);
}

#[test]
fn injection_no_op_when_only_one_action() {
    let actions = vec![AgentAction::Respond {
        content: "{{last_output}}".to_string(),
    }];
    let before = actions_as_json(&actions);
    let out = inject_synthesize_answer_for_bare_placeholder_respond(actions, "x");
    assert_eq!(
        actions_as_json(&out),
        before,
        "no observation step before respond → cannot meaningfully inject"
    );
}

#[test]
fn injection_no_op_when_last_action_is_not_respond() {
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({ "command": "ls" }),
    }];
    let before = actions_as_json(&actions);
    let out = inject_synthesize_answer_for_bare_placeholder_respond(actions, "x");
    assert_eq!(actions_as_json(&out), before);
}

#[test]
fn normalizer_drops_pre_observation_synthesize_when_concrete_respond_exists() {
    let state = test_state();
    let loop_state = LoopState::new(2);
    let route = route_result(
        crate::AskMode::direct_answer(),
        false,
        OutputResponseShape::Free,
    );
    let actions = vec![
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "早出晚归皆是梦，\n一杯咖啡换人间。".to_string(),
        },
    ];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &loop_state,
        "写一首两句的打工人短诗",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 1);
    assert!(matches!(
        &normalized[0],
        AgentAction::Respond { content }
            if content == "早出晚归皆是梦，\n一杯咖啡换人间。"
    ));
}

#[test]
fn normalizer_keeps_prior_observation_synthesize_and_placeholders_concrete_respond() {
    let state = test_state();
    let mut loop_state = LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.last_output = Some("{\"ports_snapshot\":[\"0.0.0.0:22\"]}".to_string());
    let route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    let actions = vec![
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "监听端口里最值得注意的是 0.0.0.0:22。".to_string(),
        },
    ];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &loop_state,
        "看看这台机器现在有哪些端口在监听",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 2);
    assert!(matches!(
        &normalized[0],
        AgentAction::SynthesizeAnswer { evidence_refs }
            if evidence_refs == &vec!["last_output".to_string()]
    ));
    assert!(matches!(
        &normalized[1],
        AgentAction::Respond { content } if content == "{{last_output}}"
    ));
}

#[test]
fn normalizer_prefers_synthesized_scalar_equality_over_concrete_respond() {
    use crate::executor::{StepExecutionResult, StepExecutionStatus};

    let state = test_state();
    let mut loop_state = LoopState::new(3);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "config_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            json!({
                "extra": {
                    "action": "read_field",
                    "path": "/repo/package.json",
                    "field_path": "name",
                    "exists": true,
                    "value_text": "rustclaw-nl-fixture",
                    "value": "rustclaw-nl-fixture"
                }
            })
            .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "config_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            json!({
                "extra": {
                    "action": "read_field",
                    "path": "/repo/Cargo.toml",
                    "field_path": "package.name",
                    "exists": true,
                    "value_text": "clawd",
                    "value": "clawd"
                }
            })
            .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RecentScalarEqualityCheck;
    route.output_contract.requires_content_evidence = true;
    let actions = vec![
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["step_1".to_string(), "step_2".to_string()],
        },
        AgentAction::Respond {
            content: "rustclaw-nl-fixture vs clawd different".to_string(),
        },
    ];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &loop_state,
        "compare two structured fields",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 2);
    assert!(matches!(
        &normalized[0],
        AgentAction::SynthesizeAnswer { evidence_refs }
            if evidence_refs == &vec!["step_1".to_string(), "step_2".to_string()]
    ));
    assert!(matches!(
        &normalized[1],
        AgentAction::Respond { content } if content == "{{last_output}}"
    ));
}

#[test]
fn normalizer_keeps_observation_backed_synthesize_before_respond() {
    let state = test_state();
    let loop_state = LoopState::new(2);
    let route = route_result(
        crate::AskMode::planner_execute_plain(),
        false,
        OutputResponseShape::Free,
    );
    let actions = vec![
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({ "command": "pwd" }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];
    let before = actions_as_json(&actions);

    let normalized =
        normalize_planned_actions(&state, Some(&route), &loop_state, "执行 pwd", None, actions);

    assert_eq!(actions_as_json(&normalized), before);
}

/// §F1：`has_pre_observation_structured_output_shape` 结构形态检测覆盖。
#[test]
fn pre_observation_structured_output_shape_recognizes_listing_shapes() {
    // 真实 adv08 复现：list_dir 还没跑，respond 编出 5 行 numbered 列表 + 路径。
    let adv08 = "prompts 目录前 5 个文件名：\n1. prompts/skills\n2. prompts/agents\n3. prompts/system\n4. prompts/user\n5. prompts/layers";
    assert!(has_pre_observation_structured_output_shape(adv08));

    // 多行 + 文件后缀，但没编号。
    let multi_paths = "Cargo.toml\nCargo.lock\nREADME.md\nLICENSE";
    assert!(has_pre_observation_structured_output_shape(multi_paths));

    // 结构化字段标签。
    assert!(has_pre_observation_structured_output_shape(
        "result: 42\ncount: 3"
    ));

    // 一句正常文本 → 不命中。
    assert!(!has_pre_observation_structured_output_shape(
        "好的，正在查询，请稍候。"
    ));
    // {{last_output}} 占位符 → 不命中（应由 synthesize 注入兜底处理）。
    assert!(!has_pre_observation_structured_output_shape(
        "{{last_output}}"
    ));
    // 只有一行短回复 → 不命中。
    assert!(!has_pre_observation_structured_output_shape("yes"));
}

/// §F1：rewrite 触发条件 —— round 1 + 上一步 CallSkill + Respond 含枚举。
#[test]
fn rewrite_pre_observation_rewrites_concrete_respond_after_call_skill() {
    let loop_state = LoopState::new(2);
    assert!(loop_state.executed_step_results.is_empty());
    assert!(loop_state.last_output.is_none());

    let actions = vec![
            AgentAction::CallSkill {
                skill: "list_dir".to_string(),
                args: json!({"path": "/home/guagua/rustclaw/prompts"}),
            },
            AgentAction::Respond {
                content: "prompts 目录前 5 个文件名：\n1. prompts/skills\n2. prompts/agents\n3. prompts/system\n4. prompts/user\n5. prompts/layers".to_string(),
            },
        ];
    let out = rewrite_pre_observation_concrete_respond_to_placeholder(
        &test_state(),
        None,
        &loop_state,
        actions,
    );
    match out.last().expect("should have a last action") {
        AgentAction::Respond { content } => {
            assert_eq!(
                content, "{{last_output}}",
                "concrete content must be replaced with placeholder"
            );
        }
        other => panic!("last action should remain Respond, got: {:?}", other),
    }
}

#[test]
fn rewrite_pre_observation_uses_output_contract_without_shape_matching() {
    let loop_state = LoopState::new(2);
    let route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    let actions = vec![
        AgentAction::CallSkill {
            skill: "service_control".to_string(),
            args: json!({"action": "status", "service": "rustclaw"}),
        },
        AgentAction::Respond {
            content: "服务运行正常，可以继续使用。".to_string(),
        },
    ];

    let out = rewrite_pre_observation_concrete_respond_to_placeholder(
        &test_state(),
        Some(&route),
        &loop_state,
        actions,
    );

    assert!(matches!(
        out.last(),
        Some(AgentAction::Respond { content }) if content == "{{last_output}}"
    ));
}

/// §F1：执行过任何 step 后不再触发（避免误改 round 2+ 的合法 grounded respond）。
#[test]
fn rewrite_pre_observation_no_op_after_any_step_executed() {
    use crate::executor::{StepExecutionResult, StepExecutionStatus};
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "s1".to_string(),
        skill: "list_dir".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some("foo\nbar".to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    loop_state.last_output = Some("foo\nbar".to_string());

    let actions = vec![
        AgentAction::CallSkill {
            skill: "list_dir".to_string(),
            args: json!({"path": "/x"}),
        },
        AgentAction::Respond {
            content: "1. foo\n2. bar".to_string(),
        },
    ];
    let before = actions.clone();
    let after = rewrite_pre_observation_concrete_respond_to_placeholder(
        &test_state(),
        None,
        &loop_state,
        actions,
    );
    assert_eq!(actions_as_json(&before), actions_as_json(&after));
}

/// §F1：Respond 内容是合法占位符或短确认时不触发。
#[test]
fn rewrite_pre_observation_no_op_for_placeholder_or_short_ack() {
    let loop_state = LoopState::new(2);
    for content in ["{{last_output}}", "好的", "稍候，正在执行"] {
        let actions = vec![
            AgentAction::CallSkill {
                skill: "run_cmd".to_string(),
                args: json!({"command": "ls"}),
            },
            AgentAction::Respond {
                content: content.to_string(),
            },
        ];
        let before = actions.clone();
        let after = rewrite_pre_observation_concrete_respond_to_placeholder(
            &test_state(),
            None,
            &loop_state,
            actions,
        );
        assert_eq!(
            actions_as_json(&before),
            actions_as_json(&after),
            "should not rewrite for content={:?}",
            content
        );
    }
}

#[test]
fn rewrite_terminal_placeholder_respond_inserts_synthesize_answer() {
    let loop_state = LoopState::new(2);
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "read_range",
                "path": "service_notes.md",
                "mode": "head",
                "n": 20
            }),
        },
        AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: json!({ "path": "README.md" }),
        },
        AgentAction::Respond {
            content: "先看 {{s1.output}}，再说明 {{s2.output}} 的作用".to_string(),
        },
    ];

    let rewritten = rewrite_terminal_placeholder_respond_to_synthesize_answer(&loop_state, actions);

    assert_eq!(rewritten.len(), 4);
    assert!(matches!(
        &rewritten[0],
        AgentAction::CallSkill { skill, .. } if skill == "system_basic"
    ));
    assert!(matches!(
        &rewritten[1],
        AgentAction::CallSkill { skill, .. } if skill == "read_file"
    ));
    assert!(matches!(
        &rewritten[2],
        AgentAction::SynthesizeAnswer { evidence_refs }
            if evidence_refs.as_slice()
                == ["s1.output".to_string(), "s2.output".to_string()].as_slice()
    ));
    assert!(matches!(
        &rewritten[3],
        AgentAction::Respond { content } if content == "{{last_output}}"
    ));
}

#[test]
fn normalized_multi_command_failure_summary_preserves_all_observations() {
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    let loop_state = LoopState::new(1);
    let actions = vec![
            AgentAction::CallSkill {
                skill: "run_cmd".to_string(),
                args: json!({"command": "echo THINK_BREAK_CN"}),
            },
            AgentAction::CallSkill {
                skill: "run_cmd".to_string(),
                args: json!({"command": "definitely_missing_command_minimax_think_24690"}),
            },
            AgentAction::Respond {
                content: "执行结果总结：\n\n- **echo THINK_BREAK_CN** -> 成功，输出：{{s1.output}}\n- **definitely_missing_command_minimax_think_24690** -> 失败，输出：{{s2.output}}"
                    .to_string(),
            },
        ];

    let state = test_state_with_enabled_skills(&["run_cmd"]);
    let normalized = normalize_planned_actions_with_original(
            &state,
            Some(&route),
            &loop_state,
            "先执行 echo THINK_BREAK_CN，再执行 definitely_missing_command_minimax_think_24690，然后总结成功和失败分别是什么",
            None,
            Some("/home/guagua/rustclaw"),
            actions,
        );

    assert_eq!(
        actions_as_json(&normalized),
        json!([
            {
                "type": "call_skill",
                "skill": "run_cmd",
                "args": {
                    "command": "echo THINK_BREAK_CN",
                    "_clawd_continue_on_error": true
                }
            },
            {
                "type": "call_skill",
                "skill": "run_cmd",
                "args": {
                    "command": "definitely_missing_command_minimax_think_24690",
                    "_clawd_continue_on_error": true
                }
            },
            {
                "type": "synthesize_answer",
                "evidence_refs": ["step_1", "step_2"]
            },
            {
                "type": "respond",
                "content": "{{last_output}}"
            }
        ])
    );
    assert_eq!(normalized.len(), 4);
    assert!(matches!(
        &normalized[0],
        AgentAction::CallSkill { skill, args }
            if skill == "run_cmd"
                && args.get("command").and_then(Value::as_str) == Some("echo THINK_BREAK_CN")
    ));
    assert!(matches!(
        &normalized[1],
        AgentAction::CallSkill { skill, args }
            if skill == "run_cmd"
                && args.get("command").and_then(Value::as_str)
                    == Some("definitely_missing_command_minimax_think_24690")
    ));
    assert_eq!(
        super::super::action_args(&normalized[0])
            .and_then(|args| args.get("_clawd_continue_on_error"))
            .and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        super::super::action_args(&normalized[1])
            .and_then(|args| args.get("_clawd_continue_on_error"))
            .and_then(Value::as_bool),
        Some(true)
    );
    assert!(matches!(
        &normalized[2],
        AgentAction::SynthesizeAnswer { evidence_refs }
            if evidence_refs.as_slice()
                == ["step_1".to_string(), "step_2".to_string()].as_slice()
    ));
    assert!(matches!(
        &normalized[3],
        AgentAction::Respond { content } if content == "{{last_output}}"
    ));
}

#[test]
fn normalized_run_cmd_observation_sequence_marks_continue_on_error() {
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::CommandOutputSummary;
    let loop_state = LoopState::new(1);
    let actions = vec![
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({"command": "printenv PATH"}),
        },
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({"command": "definitely_absent_command_for_sequence_marker"}),
        },
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({"command": "uname -s"}),
        },
    ];

    let state = test_state_with_enabled_skills(&["run_cmd"]);
    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &loop_state,
        "Run the listed command sequence and report each result.",
        None,
        Some("/home/guagua/rustclaw"),
        actions,
    );

    assert!(normalized.len() >= 3);
    for action in normalized.iter().take(3) {
        let args = super::super::action_args(action).expect("run_cmd args");
        assert_eq!(
            args.get("_clawd_continue_on_error")
                .and_then(Value::as_bool),
            Some(true)
        );
    }
}

#[test]
fn normalized_raw_command_output_sequence_does_not_mark_continue_on_error() {
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    let loop_state = LoopState::new(1);
    let actions = vec![
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({"command": "echo BEFORE_CHANGE"}),
        },
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({"command": "definitely_absent_command_for_raw_sequence"}),
        },
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({"command": "echo AFTER_CHANGE_OLD"}),
        },
    ];

    let state = test_state_with_enabled_skills(&["run_cmd"]);
    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &loop_state,
        "Run these commands in order.",
        None,
        Some("/home/guagua/rustclaw"),
        actions,
    );

    assert!(normalized.len() >= 3);
    for action in normalized.iter().take(3) {
        let args = super::super::action_args(action).expect("run_cmd args");
        assert_eq!(args.get("_clawd_continue_on_error"), None);
    }
}

#[test]
fn normalized_run_cmd_mutation_sequence_does_not_mark_continue_on_error() {
    let route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    let loop_state = LoopState::new(1);
    let actions = vec![
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({"command": "mkdir tmp_sequence_marker"}),
        },
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({"command": "pwd"}),
        },
    ];

    let state = test_state_with_enabled_skills(&["run_cmd"]);
    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &loop_state,
        "Run this setup command and then inspect the current directory.",
        None,
        Some("/home/guagua/rustclaw"),
        actions,
    );

    assert!(normalized.len() >= 2);
    for action in normalized.iter().take(2) {
        let args = super::super::action_args(action).expect("run_cmd args");
        assert_eq!(args.get("_clawd_continue_on_error"), None);
    }
}

#[test]
fn planner_introduced_tail_run_cmd_rewrites_to_fs_basic_read_range() {
    let route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    let loop_state = LoopState::new(1);
    let actions = vec![
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({"command": "tail -n 3 /home/guagua/rustclaw/logs/clawd.run.log"}),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
    ];

    let state = test_state_with_enabled_skills(&["system_basic", "run_cmd"]);
    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &loop_state,
        "查看 logs/clawd.run.log 最后 3 行，只做简短概述。",
        None,
        Some("/home/guagua/rustclaw/logs/clawd.run.log"),
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "read_text_range");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some("/home/guagua/rustclaw/logs/clawd.run.log")
    );
    assert_eq!(args.get("mode").and_then(Value::as_str), Some("tail"));
    assert_eq!(args.get("n").and_then(Value::as_u64), Some(3));
}

#[test]
fn content_excerpt_summary_tail_run_cmd_does_not_insert_default_head_read() {
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    let loop_state = LoopState::new(1);
    let path = "/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/logs/model_io.log";
    let actions = vec![
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({"command": format!("tail -n 4 {path}")}),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let state = test_state_with_enabled_skills(&["fs_basic", "run_cmd"]);
    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &loop_state,
        "看一下日志最后 4 行，再一句话说有没有失败后恢复。",
        None,
        Some(path),
        actions,
    );

    let reads: Vec<&Value> = normalized
        .iter()
        .filter_map(|action| {
            planned_call(action).and_then(|(tool, args)| {
                (tool == "fs_basic"
                    && args.get("action").and_then(Value::as_str) == Some("read_text_range"))
                .then_some(args)
            })
        })
        .collect();
    assert_eq!(reads.len(), 1, "normalized actions: {normalized:?}");
    assert_eq!(reads[0].get("path").and_then(Value::as_str), Some(path));
    assert_eq!(reads[0].get("mode").and_then(Value::as_str), Some("tail"));
    assert_eq!(reads[0].get("n").and_then(Value::as_u64), Some(4));
}

#[test]
fn planner_introduced_echo_append_run_cmd_rewrites_to_fs_basic_append_text() {
    let route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    let loop_state = LoopState::new(1);
    let actions = vec![AgentAction::CallTool {
        tool: "run_cmd".to_string(),
        args: json!({
            "command": "echo \"beta\" >> document/nl_tool200/group_02/memo.txt",
            "cwd": "/home/guagua/rustclaw"
        }),
    }];

    let state = test_state_with_enabled_skills(&["fs_basic", "run_cmd"]);
    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &loop_state,
        "Append beta to the known memo file.",
        None,
        Some("/home/guagua/rustclaw/document/nl_tool200/group_02/memo.txt"),
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "append_text");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some("/home/guagua/rustclaw/document/nl_tool200/group_02/memo.txt")
    );
    assert_eq!(args.get("content").and_then(Value::as_str), Some("beta\n"));
}

#[test]
fn planner_introduced_simple_fs_run_cmd_sequence_rewrites_to_fs_basic_lifecycle() {
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ExecutionFailedStep;
    route.output_contract.locator_hint = "tmp/nl_basic_skill_coverage_case".to_string();
    let loop_state = LoopState::new(2);
    let actions = vec![
        AgentAction::CallTool {
            tool: "run_cmd".to_string(),
            args: json!({"command": "mkdir -p tmp/nl_basic_skill_coverage_case"}),
        },
        AgentAction::CallTool {
            tool: "run_cmd".to_string(),
            args: json!({"command": "echo alpha > tmp/nl_basic_skill_coverage_case/note.txt"}),
        },
        AgentAction::CallTool {
            tool: "run_cmd".to_string(),
            args: json!({"command": "echo beta >> tmp/nl_basic_skill_coverage_case/note.txt"}),
        },
        AgentAction::CallTool {
            tool: "run_cmd".to_string(),
            args: json!({"command": "cat tmp/nl_basic_skill_coverage_case/note.txt"}),
        },
        AgentAction::CallTool {
            tool: "run_cmd".to_string(),
            args: json!({"command": "rm -rf tmp/nl_basic_skill_coverage_case"}),
        },
    ];

    let state = test_state_with_enabled_skills(&["fs_basic", "run_cmd"]);
    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &loop_state,
        "scratch filesystem lifecycle through planner tools",
        None,
        None,
        actions,
    );
    let fs_actions = normalized
        .iter()
        .filter_map(planned_call)
        .filter(|(tool, _)| *tool == "fs_basic")
        .map(|(_, args)| args)
        .collect::<Vec<_>>();

    assert_eq!(fs_actions.len(), 5, "normalized actions: {normalized:?}");
    assert_eq!(
        fs_actions
            .iter()
            .filter_map(|args| args.get("action").and_then(Value::as_str))
            .collect::<Vec<_>>(),
        vec![
            "make_dir",
            "write_text",
            "append_text",
            "read_text_range",
            "remove_path"
        ]
    );
    assert_eq!(
        fs_actions[4].get("target_kind").and_then(Value::as_str),
        Some("directory")
    );
    assert_eq!(
        fs_actions[4].get("recursive").and_then(Value::as_bool),
        Some(true)
    );

    let steps = fs_actions
        .iter()
        .enumerate()
        .map(|(idx, args)| crate::PlanStep {
            step_id: format!("step_{}", idx + 1),
            action_type: "call_tool".to_string(),
            skill: "fs_basic".to_string(),
            args: (*args).clone(),
            depends_on: Vec::new(),
            why: String::new(),
        })
        .collect::<Vec<_>>();
    let effective =
        crate::agent_engine::effective_filesystem_lifecycle_output_contract_for_plan_steps(
            &state, &route, &steps,
        )
        .expect("scratch lifecycle should upgrade execution_failed_step contract");
    assert_eq!(
        effective.semantic_kind,
        OutputSemanticKind::FilesystemMutationResult
    );
}

#[test]
fn user_supplied_tail_command_stays_run_cmd() {
    let route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    let loop_state = LoopState::new(1);
    let command = "tail -n 3 logs/clawd.run.log";
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": command}),
    }];

    let state = test_state_with_enabled_skills(&["system_basic", "run_cmd"]);
    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &loop_state,
        "执行 tail -n 3 logs/clawd.run.log",
        Some("执行 tail -n 3 logs/clawd.run.log"),
        Some("/home/guagua/rustclaw/logs/clawd.run.log"),
        actions,
    );

    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "run_cmd");
            assert_eq!(args.get("command").and_then(Value::as_str), Some(command));
        }
        other => panic!("expected preserved run_cmd action, got {other:?}"),
    }
}

#[test]
fn planner_introduced_find_extension_dirs_rewrites_to_fs_basic() {
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryNames;
    let loop_state = LoopState::new(1);
    let command = r#"find . -name '*.sh' -type f -exec dirname {} \; | sort -u"#;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({"command": command}),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
    ];

    let state = test_state_with_enabled_skills(&["fs_basic", "run_cmd"]);
    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &loop_state,
        "查找当前仓库里所有 sh 脚本所在的目录，去重后列出来",
        None,
        Some("/home/guagua/rustclaw"),
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "find_entries");
    assert_eq!(args.get("root").and_then(Value::as_str), Some("."));
    assert_eq!(args.get("extension").and_then(Value::as_str), Some("sh"));
    assert_eq!(args.get("files_only").and_then(Value::as_bool), Some(true));
    assert_eq!(args.get("recursive").and_then(Value::as_bool), Some(true));
}

#[test]
fn recent_artifacts_repaired_shell_listing_keeps_structured_selector() {
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "logs".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::RecentArtifactsJudgment;
    route
        .output_contract
        .self_extension
        .list_selector
        .target_kind = crate::OutputScalarCountTargetKind::File;
    route
        .output_contract
        .self_extension
        .list_selector
        .target_kind_specified = true;
    route.output_contract.self_extension.list_selector.limit = Some(2);
    route.output_contract.self_extension.list_selector.sort_by = Some("mtime_desc".to_string());
    let loop_state = LoopState::new(1);
    let actions = vec![
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({
                "command": "cd /home/guagua/rustclaw/logs && ls -1t | head -2"
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let state = test_state_with_enabled_skills(&["fs_basic", "run_cmd"]);
    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &loop_state,
        "List the 2 most recently modified files under logs.",
        None,
        Some("/home/guagua/rustclaw/logs"),
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "list_dir");
    assert_eq!(args.get("path").and_then(Value::as_str), Some("logs"));
    assert_eq!(args.get("files_only").and_then(Value::as_bool), Some(true));
    assert_eq!(args.get("dirs_only").and_then(Value::as_bool), Some(false));
    assert_eq!(args.get("names_only").and_then(Value::as_bool), Some(false));
    assert_eq!(args.get("max_entries").and_then(Value::as_u64), Some(2));
    assert_eq!(
        args.get("sort_by").and_then(Value::as_str),
        Some("mtime_desc")
    );
}

#[test]
fn planner_introduced_find_sed_parent_dirs_rewrites_to_fs_basic() {
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryNames;
    let loop_state = LoopState::new(1);
    let command =
        r#"find /home/guagua/rustclaw -name '*.sh' -type f | sed 's|/[^/]*$||' | sort -u"#;
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": command}),
    }];

    let state = test_state_with_enabled_skills(&["fs_basic", "run_cmd"]);
    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &loop_state,
        "扫描当前仓库中所有.sh文件，提取其所在目录路径并去重排序后输出",
        None,
        Some("/home/guagua/rustclaw"),
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "find_entries");
    assert_eq!(
        args.get("root").and_then(Value::as_str),
        Some("/home/guagua/rustclaw")
    );
    assert_eq!(args.get("extension").and_then(Value::as_str), Some("sh"));
    assert_eq!(args.get("files_only").and_then(Value::as_bool), Some(true));
    assert_eq!(args.get("recursive").and_then(Value::as_bool), Some(true));
}

#[test]
fn structured_find_observation_strips_redundant_shell_followup() {
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryNames;
    let loop_state = LoopState::new(1);
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "find_entries",
                "root": "/home/guagua/rustclaw",
                "ext": "sh",
                "target_kind": "file"
            }),
        },
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({
                "command": "find /home/guagua/rustclaw -name '*.sh' -exec dirname {} \\; | sort -u"
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
    ];

    let state = test_state_with_enabled_skills(&["fs_basic", "run_cmd"]);
    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &loop_state,
        "查找当前仓库里所有 sh 脚本所在的目录，去重后列出来",
        None,
        Some("/home/guagua/rustclaw"),
        actions,
    );

    assert!(normalized.iter().all(
        |action| !matches!(action, AgentAction::CallSkill { skill, .. } if skill == "run_cmd")
    ));
    assert!(normalized
        .iter()
        .all(|action| !matches!(action, AgentAction::SynthesizeAnswer { .. })));
    assert!(normalized
        .iter()
        .all(|action| planned_call_is(action, "fs_basic", "find_entries")));
}

#[test]
fn user_supplied_find_extension_command_stays_run_cmd() {
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryNames;
    let loop_state = LoopState::new(1);
    let command = r#"find . -name '*.sh' -type f -exec dirname {} \; | sort -u"#;
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": command}),
    }];

    let state = test_state_with_enabled_skills(&["fs_basic", "run_cmd"]);
    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &loop_state,
        "执行 find . -name '*.sh' -type f -exec dirname {} \\; | sort -u",
        Some("执行 find . -name '*.sh' -type f -exec dirname {} \\; | sort -u"),
        Some("/home/guagua/rustclaw"),
        actions,
    );

    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "run_cmd");
            assert_eq!(args.get("command").and_then(Value::as_str), Some(command));
        }
        other => panic!("expected preserved run_cmd action, got {other:?}"),
    }
}

#[test]
fn piped_tail_command_is_not_rewritten_to_file_tool() {
    let route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    let loop_state = LoopState::new(1);
    let command = "tail -n 3 logs/clawd.run.log | sed -n '1p'";
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": command}),
    }];

    let state = test_state_with_enabled_skills(&["system_basic", "run_cmd"]);
    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &loop_state,
        "查看日志尾部第一行",
        None,
        Some("/home/guagua/rustclaw/logs/clawd.run.log"),
        actions,
    );

    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "run_cmd");
            assert_eq!(args.get("command").and_then(Value::as_str), Some(command));
        }
        other => panic!("expected preserved piped run_cmd action, got {other:?}"),
    }
}

#[test]
fn normalized_single_sequential_run_cmd_splits_for_step_status_evidence() {
    let route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    let loop_state = LoopState::new(1);
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({
            "command": "echo THINK_BREAK_CN; definitely_missing_command_minimax_think_24690",
            "cwd": "/home/guagua/rustclaw"
        }),
    }];

    let state = test_state_with_enabled_skills(&["run_cmd"]);
    let normalized = normalize_planned_actions_with_original(
            &state,
            Some(&route),
            &loop_state,
            "执行两个命令：echo THINK_BREAK_CN 和 definitely_missing_command_minimax_think_24690，然后总结哪些成功、哪些失败",
            None,
            Some("/home/guagua/rustclaw"),
            actions,
        );

    assert_eq!(
        actions_as_json(&normalized),
        json!([
            {
                "type": "call_skill",
                "skill": "run_cmd",
                "args": {
                    "command": "echo THINK_BREAK_CN",
                    "cwd": "/home/guagua/rustclaw",
                    "_clawd_continue_on_error": true
                }
            },
            {
                "type": "call_skill",
                "skill": "run_cmd",
                "args": {
                    "command": "definitely_missing_command_minimax_think_24690",
                    "cwd": "/home/guagua/rustclaw",
                    "_clawd_continue_on_error": true
                }
            },
            {
                "type": "synthesize_answer",
                "evidence_refs": ["step_1", "step_2"]
            },
            {
                "type": "respond",
                "content": "{{last_output}}"
            }
        ])
    );
}

#[test]
fn normalized_planner_introduced_and_sequence_splits_for_step_status_evidence() {
    let route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::OneSentence,
    );
    let loop_state = LoopState::new(1);
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({
            "command": "echo BEFORE_BREAK && definitely_missing_command_rustclaw_user_ops_13579"
        }),
    }];

    let state = test_state_with_enabled_skills(&["run_cmd"]);
    let normalized = normalize_planned_actions_with_original(
            &state,
            Some(&route),
            &loop_state,
            "执行两个命令：先 echo BEFORE_BREAK，再 definitely_missing_command_rustclaw_user_ops_13579，报告哪一步失败了",
            Some(
                "先执行 echo BEFORE_BREAK，再执行 definitely_missing_command_rustclaw_user_ops_13579，只告诉我哪一步挂了",
            ),
            Some("/home/guagua/rustclaw"),
            actions,
        );

    assert_eq!(
        actions_as_json(&normalized),
        json!([
            {
                "type": "call_skill",
                "skill": "run_cmd",
                "args": {
                    "command": "echo BEFORE_BREAK",
                    "_clawd_continue_on_error": true
                }
            },
            {
                "type": "call_skill",
                "skill": "run_cmd",
                "args": {
                    "command": "definitely_missing_command_rustclaw_user_ops_13579",
                    "_clawd_continue_on_error": true
                }
            },
            {
                "type": "synthesize_answer",
                "evidence_refs": ["step_1", "step_2"]
            },
            {
                "type": "respond",
                "content": "{{last_output}}"
            }
        ])
    );
}

#[test]
fn user_supplied_and_operator_is_preserved_as_one_command() {
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": "echo BEFORE_BREAK && echo AFTER_BREAK"}),
    }];

    let rewritten = super::super::split_sequential_run_cmd_actions(
        "Run `echo BEFORE_BREAK && echo AFTER_BREAK` exactly.",
        Some("Run `echo BEFORE_BREAK && echo AFTER_BREAK` exactly."),
        actions,
    );

    assert_eq!(rewritten.len(), 1);
    assert!(matches!(
        &rewritten[0],
        AgentAction::CallSkill { args, .. }
            if args.get("command").and_then(Value::as_str)
                == Some("echo BEFORE_BREAK && echo AFTER_BREAK")
    ));
}

#[test]
fn user_supplied_or_operator_is_preserved_as_one_command() {
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": "missing_probe --version || which bash"}),
    }];

    let rewritten = super::super::split_sequential_run_cmd_actions(
        "Run `missing_probe --version || which bash` exactly.",
        Some("Run `missing_probe --version || which bash` exactly."),
        actions,
    );

    assert_eq!(rewritten.len(), 1);
    assert!(matches!(
        &rewritten[0],
        AgentAction::CallSkill { args, .. }
            if args.get("command").and_then(Value::as_str)
                == Some("missing_probe --version || which bash")
    ));
}

#[test]
fn user_supplied_semicolon_command_is_preserved_as_one_command() {
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": "printf problem >&2; exit 7"}),
    }];

    let rewritten = super::super::split_sequential_run_cmd_actions(
        "执行命令 `printf problem >&2; exit 7`，报告退出码和 stderr 错误输出。",
        Some("执行命令 printf problem >&2; exit 7，告诉我退出码和错误输出。"),
        actions,
    );

    assert_eq!(rewritten.len(), 1);
    assert!(matches!(
        &rewritten[0],
        AgentAction::CallSkill { args, .. }
            if args.get("command").and_then(Value::as_str)
                == Some("printf problem >&2; exit 7")
    ));
}

#[test]
fn planner_introduced_or_operator_becomes_first_visible_attempt() {
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({
            "command": "missing_probe --version 2>/dev/null || which bash",
            "_clawd_continue_on_error": true,
            "_clawd_literal_command": true
        }),
    }];

    let rewritten = super::super::split_sequential_run_cmd_actions(
        "Run missing_probe --version. If it is missing, run which bash.",
        Some("Run missing_probe --version. If it is missing, run which bash."),
        actions,
    );

    assert_eq!(rewritten.len(), 1);
    assert!(matches!(
        &rewritten[0],
        AgentAction::CallSkill { args, .. }
            if args.get("command").and_then(Value::as_str)
                == Some("missing_probe --version 2>/dev/null")
                && args.get("_clawd_continue_on_error").is_none()
                && args.get("_clawd_literal_command").is_none()
    ));
}

#[test]
fn planner_introduced_and_operator_can_split_when_user_did_not_supply_it() {
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": "echo BEFORE_BREAK && echo AFTER_BREAK"}),
    }];

    let rewritten = super::super::split_sequential_run_cmd_actions(
        "Run echo BEFORE_BREAK, then run echo AFTER_BREAK.",
        Some("Run echo BEFORE_BREAK, then run echo AFTER_BREAK."),
        actions,
    );

    assert_eq!(rewritten.len(), 2);
    assert!(matches!(
        &rewritten[0],
        AgentAction::CallSkill { args, .. }
            if args.get("command").and_then(Value::as_str) == Some("echo BEFORE_BREAK")
    ));
    assert!(matches!(
        &rewritten[1],
        AgentAction::CallSkill { args, .. }
            if args.get("command").and_then(Value::as_str) == Some("echo AFTER_BREAK")
    ));
}

#[test]
fn shell_sequence_splitter_ignores_quoted_semicolons_and_stateful_prefixes() {
    assert_eq!(
        super::super::split_shell_sequence_command_with_policy("echo a; echo b", false),
        Some(vec!["echo a".to_string(), "echo b".to_string()])
    );
    assert_eq!(
        super::super::split_shell_sequence_command_with_policy("printf 'a;b\\n'", false),
        None
    );
    assert_eq!(
        super::super::split_shell_sequence_command_with_policy("cd /tmp; pwd", false),
        None
    );
}

#[test]
fn shell_sequence_splitter_preserves_assignment_state() {
    let command = concat!(
        "F='document/nl_ops_http_repair_demo/index.html'; ",
        "sed -i 's/ops-repair-[a-zA-Z0-9_-]*/ops-repair-ok/g' \"$F\" && ",
        "echo '--- after replace ---' && ",
        "cat \"$F\""
    );

    assert_eq!(
        super::super::split_shell_sequence_command_with_policy(command, true),
        None
    );
}

#[test]
fn rewrite_terminal_expression_placeholder_respond_inserts_synthesize_answer() {
    let loop_state = LoopState::new(2);
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({"action": "extract_field", "path": "package.json", "field_path": "name"}),
        },
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({"action": "extract_field", "path": "Cargo.toml", "field_path": "package.name"}),
        },
        AgentAction::Respond {
            content: "name={{s1}}; crate={{s2}}; same={{s1 == s2 ? 'yes' : 'no'}}".to_string(),
        },
    ];

    let rewritten = rewrite_terminal_placeholder_respond_to_synthesize_answer(&loop_state, actions);

    assert_eq!(rewritten.len(), 4);
    assert!(matches!(
        &rewritten[2],
        AgentAction::SynthesizeAnswer { evidence_refs }
            if evidence_refs.as_slice() == ["s1".to_string(), "s2".to_string()].as_slice()
    ));
    assert!(matches!(
        &rewritten[3],
        AgentAction::Respond { content } if content == "{{last_output}}"
    ));
}

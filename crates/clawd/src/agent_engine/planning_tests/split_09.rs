use super::*;

#[test]
fn execution_failed_step_prefixed_bare_sequence_gets_multi_run_cmd_plan() {
    let mut state = test_state_with_enabled_skills(&["run_cmd"]);
    state.policy.command_intent.execute_prefixes = vec!["执行".to_string()];
    state.policy.command_intent.standalone_commands = vec!["pwd".to_string()];
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ExecutionFailedStep;
    route.output_contract.requires_content_evidence = true;
    let loop_state = LoopState::new(1);
    let request =
        "先执行 pwd，再执行 definitely_missing_command_rustclaw_67890，然后总结成功和失败";

    let plan = explicit_command_deterministic_plan_result(
        &state,
        "run command sequence and summarize success/failure",
        Some(&route),
        &loop_state,
        request,
        None,
    )
    .expect("prefixed command sequence should produce deterministic run_cmd plan");

    assert_eq!(plan.steps.len(), 4);
    assert_eq!(
        plan.steps[0].args.get("command").and_then(Value::as_str),
        Some("pwd")
    );
    assert_eq!(
        plan.steps[1].args.get("command").and_then(Value::as_str),
        Some("definitely_missing_command_rustclaw_67890")
    );
    for step in plan.steps.iter().take(2) {
        assert_eq!(step.skill, "run_cmd");
        assert_eq!(step.args.get(CLAWD_LITERAL_COMMAND_ARG), Some(&json!(true)));
        assert_eq!(
            step.args.get(CLAWD_CONTINUE_ON_ERROR_ARG),
            Some(&json!(true))
        );
    }
    assert_eq!(
        plan.steps[2].args,
        json!({"evidence_refs": ["step_1", "step_2"]})
    );
    assert_eq!(
        plan.steps[3].args.get("content").and_then(Value::as_str),
        Some("{{last_output}}")
    );
}

#[test]
fn execution_failed_step_prefixed_echo_sequence_counts_as_explicit_command_request() {
    let mut state = test_state_with_enabled_skills(&["run_cmd"]);
    state.policy.command_intent.execute_prefixes = vec!["执行".to_string()];
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ExecutionFailedStep;
    route.output_contract.requires_content_evidence = true;
    let loop_state = LoopState::new(1);
    let request = "先执行 echo BEFORE_BREAK，再执行 definitely_missing_command_rustclaw_67890，再执行 echo AFTER_BREAK_67890";

    assert!(super::super::explicit_command_request_present(
        &state.policy.command_intent,
        request,
        Some(&route)
    ));
    let plan = explicit_command_deterministic_plan_result(
        &state,
        "run command sequence and continue after failure",
        Some(&route),
        &loop_state,
        request,
        None,
    )
    .expect("explicit failed-step command sequence should not fall through to auto-locator");

    assert_eq!(plan.steps.len(), 5);
    assert_eq!(
        plan.steps[0].args.get("command").and_then(Value::as_str),
        Some("echo BEFORE_BREAK")
    );
    assert_eq!(
        plan.steps[1].args.get("command").and_then(Value::as_str),
        Some("definitely_missing_command_rustclaw_67890")
    );
    assert_eq!(
        plan.steps[2].args.get("command").and_then(Value::as_str),
        Some("echo AFTER_BREAK_67890")
    );
    for step in plan.steps.iter().take(3) {
        assert_eq!(step.skill, "run_cmd");
        assert_eq!(step.args.get(CLAWD_LITERAL_COMMAND_ARG), Some(&json!(true)));
        assert_eq!(step.args.get(CLAWD_CONTINUE_ON_ERROR_ARG), None);
    }
    assert_eq!(
        plan.steps[3].args,
        json!({"evidence_refs": ["step_1", "step_2", "step_3"]})
    );
    assert_eq!(
        plan.steps[4].args.get("content").and_then(Value::as_str),
        Some("{{last_output}}")
    );
}

#[test]
fn conditional_step_update_limits_current_explicit_command_plan_to_pre_update_steps() {
    let state = test_state_with_enabled_skills(&["run_cmd"]);
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::CommandOutputSummary;
    route.output_contract.requires_content_evidence = true;
    let loop_state = LoopState::new(1);
    let request = "Run `echo BEFORE_CHANGE_EN`, then `definitely_missing_command_rustclaw_change_24683`, then `echo AFTER_CHANGE_OLD_EN`; if I later say continue with a change, replace the last step with `echo AFTER_CHANGE_NEW_EN`.";
    let turn_analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::TaskRequest),
        target_task_policy: None,
        should_interrupt_active_run: false,
        state_patch: Some(json!({
            "conditional_step_update": {
                "step_to_modify": 3,
                "original_command": "echo AFTER_CHANGE_OLD_EN",
                "replacement_command": "echo AFTER_CHANGE_NEW_EN",
                "trigger_condition": "user_says_continue_after_failure"
            }
        })),
        attachment_processing_required: false,
    };

    let plan = explicit_command_deterministic_plan_result(
        &state,
        "run failed command prefix and remember conditional continuation",
        Some(&route),
        &loop_state,
        request,
        Some(&turn_analysis),
    )
    .expect("conditional continuation should still produce deterministic run_cmd plan");

    assert_eq!(plan.steps.len(), 4);
    assert_eq!(
        plan.steps[0].args.get("command").and_then(Value::as_str),
        Some("echo BEFORE_CHANGE_EN")
    );
    assert_eq!(
        plan.steps[1].args.get("command").and_then(Value::as_str),
        Some("definitely_missing_command_rustclaw_change_24683")
    );
    for step in plan.steps.iter().take(2) {
        assert_eq!(step.args.get(CLAWD_CONTINUE_ON_ERROR_ARG), None);
    }
    assert_eq!(plan.steps[2].action_type, "synthesize_answer");
    assert_eq!(
        plan.steps[2].args,
        json!({"evidence_refs": ["step_1", "step_2"]})
    );
    assert_eq!(plan.steps[3].action_type, "respond");
    let executed = plan
        .steps
        .iter()
        .filter_map(|step| step.args.get("command").and_then(Value::as_str))
        .collect::<Vec<_>>();
    assert!(!executed.contains(&"echo AFTER_CHANGE_OLD_EN"));
    assert!(!executed.contains(&"echo AFTER_CHANGE_NEW_EN"));
}

#[test]
fn multi_explicit_run_cmd_plan_marks_literal_commands_without_continue_on_error() {
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route.output_contract.requires_content_evidence = true;
    let loop_state = LoopState::new(1);
    let actions = vec![
        AgentAction::CallTool {
            tool: "run_cmd".to_string(),
            args: json!({"command": "echo BEFORE_CHANGE", "cwd": "/home/guagua/rustclaw"}),
        },
        AgentAction::CallTool {
            tool: "run_cmd".to_string(),
            args: json!({
                "command": "definitely_missing_command_rustclaw_change_24682",
                "cwd": "/home/guagua/rustclaw"
            }),
        },
    ];

    let state = test_state_with_enabled_skills(&["run_cmd"]);
    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &loop_state,
        "Execute command sequence",
        Some("先执行 echo BEFORE_CHANGE，再执行 definitely_missing_command_rustclaw_change_24682，再执行 echo AFTER_CHANGE_OLD；如果稍后继续则替换最后一步。"),
        Some("/home/guagua/rustclaw"),
        actions,
    );

    assert_eq!(normalized.len(), 2);
    for action in &normalized {
        let args = super::super::action_args(action).expect("run_cmd args");
        assert_eq!(args.get(CLAWD_LITERAL_COMMAND_ARG), Some(&json!(true)));
        assert_eq!(args.get(CLAWD_CONTINUE_ON_ERROR_ARG), None);
    }
}

#[test]
fn explicit_configured_command_with_followup_skips_single_step_fast_path() {
    let mut state = test_state_with_enabled_skills(&["run_cmd"]);
    state.policy.command_intent.execute_prefixes = vec!["run ".to_string()];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route.output_contract.requires_content_evidence = true;
    let loop_state = LoopState::new(1);
    let request =
        "Run pwd, then run definitely_missing_command_rustclaw_english_67890, then summarize.";

    assert_eq!(
        super::super::explicit_command_segment(&state.policy.command_intent, request).as_deref(),
        Some("pwd")
    );
    assert!(explicit_command_deterministic_plan_result(
        &state,
        "run compound explicit commands",
        Some(&route),
        &loop_state,
        request,
        None,
    )
    .is_none());
}

#[test]
fn prefixed_path_command_with_structural_args_before_freeform_tail_is_detected() {
    let root = TempDirGuard::new("prefixed_path_command_tail");
    fs::write(root.path.join("uname"), "").expect("write command marker");

    assert_eq!(
        super::super::path_command_segment_before_freeform_tail_with_path_env(
            "uname -a and tell me the result",
            Some(root.path.as_os_str()),
        ),
        Some("uname -a")
    );
    assert!(
        super::super::path_command_segment_before_freeform_tail_with_path_env(
            "uname and tell me the result",
            Some(root.path.as_os_str()),
        )
        .is_none()
    );
}

#[test]
fn explicit_configured_path_command_with_structural_args_is_preserved() {
    let path_env = std::env::var_os("PATH");
    if !super::super::command_token_resolves_in_path("uname", path_env.as_deref()) {
        return;
    }
    let mut state = test_state_with_enabled_skills(&["run_cmd"]);
    state.policy.command_intent.execute_prefixes = vec!["please run ".to_string()];

    assert_eq!(
        super::super::explicit_command_segment(
            &state.policy.command_intent,
            "please run uname -a and tell me the result",
        )
        .as_deref(),
        Some("uname -a")
    );
}

#[test]
fn explicit_configured_command_without_followup_keeps_single_step_fast_path() {
    let mut state = test_state_with_enabled_skills(&["run_cmd"]);
    state.policy.command_intent.execute_prefixes = vec!["run ".to_string()];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route.output_contract.requires_content_evidence = true;
    let loop_state = LoopState::new(1);
    let request = "Run pwd,";

    let plan = explicit_command_deterministic_plan_result(
        &state,
        "run explicit command",
        Some(&route),
        &loop_state,
        request,
        None,
    )
    .expect("single configured command should keep deterministic plan");

    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].skill, "run_cmd");
    assert_eq!(
        plan.steps[0].args.get("command").and_then(Value::as_str),
        Some("pwd")
    );
}

#[test]
fn explicit_configured_command_inside_clause_is_detected() {
    let mut state = test_state_with_enabled_skills(&["run_cmd"]);
    state.policy.command_intent.execute_prefixes = vec!["请执行".to_string(), "执行".to_string()];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route.output_contract.requires_content_evidence = true;
    let loop_state = LoopState::new(1);
    let request = "先别管方案，请执行 pwd，只输出命令结果。";

    assert_eq!(
        super::super::explicit_command_segment(&state.policy.command_intent, request).as_deref(),
        Some("pwd")
    );
    let plan = explicit_command_deterministic_plan_result(
        &state,
        "run explicit command",
        Some(&route),
        &loop_state,
        request,
        None,
    )
    .expect("configured command in a later clause should produce run_cmd plan");

    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].skill, "run_cmd");
    assert_eq!(
        plan.steps[0].args.get("command").and_then(Value::as_str),
        Some("pwd")
    );
}

#[test]
fn embedded_standalone_command_with_structural_args_keeps_single_step_fast_path() {
    let mut state = test_state_with_enabled_skills(&["run_cmd"]);
    state.policy.command_intent.execute_prefixes = vec!["执行".to_string()];
    state.policy.command_intent.standalone_commands = vec!["pwd".to_string()];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route.output_contract.requires_content_evidence = true;
    let loop_state = LoopState::new(1);
    let request = "运行 pwd -P，只返回物理工作目录路径";

    assert_eq!(
        super::super::explicit_command_segment(&state.policy.command_intent, request).as_deref(),
        Some("pwd -P")
    );
    let plan = explicit_command_deterministic_plan_result(
        &state,
        "run explicit command",
        Some(&route),
        &loop_state,
        request,
        None,
    )
    .expect("embedded standalone command should produce run_cmd plan");

    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].skill, "run_cmd");
    assert_eq!(
        plan.steps[0].args.get("command").and_then(Value::as_str),
        Some("pwd -P")
    );
}

#[test]
fn embedded_standalone_command_sequence_uses_configured_command_tokens() {
    let mut state = test_state_with_enabled_skills(&["run_cmd"]);
    state.policy.command_intent.execute_prefixes = vec!["执行".to_string()];
    state.policy.command_intent.standalone_commands = vec!["pwd".to_string(), "whoami".to_string()];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route.output_contract.requires_content_evidence = true;
    let loop_state = LoopState::new(1);
    let request = "请依次执行 pwd 和 whoami，直接输出两个命令结果，每个结果一行，不要总结";

    assert_eq!(
        super::super::explicit_command_segment(&state.policy.command_intent, request).as_deref(),
        Some("pwd; whoami")
    );
    let plan = explicit_command_deterministic_plan_result(
        &state,
        "run explicit command sequence",
        Some(&route),
        &loop_state,
        request,
        None,
    )
    .expect("configured command sequence should produce one run_cmd plan");

    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].skill, "run_cmd");
    assert_eq!(
        plan.steps[0].args.get("command").and_then(Value::as_str),
        Some("pwd; whoami")
    );
}

#[test]
fn command_output_summary_explicit_command_plan_synthesizes_configured_sequence() {
    let mut state = test_state_with_enabled_skills(&["run_cmd"]);
    state.policy.command_intent.execute_prefixes = vec!["执行".to_string()];
    state.policy.command_intent.standalone_commands = vec!["pwd".to_string(), "whoami".to_string()];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::CommandOutputSummary;
    route.output_contract.requires_content_evidence = true;
    let loop_state = LoopState::new(1);
    let request = "先执行 whoami，再执行 pwd，然后把结果用一句自嘲签名回复我";

    assert_eq!(
        super::super::configured_distinct_standalone_command_sequence_from_text(
            &state.policy.command_intent,
            request
        )
        .as_deref(),
        Some("whoami; pwd")
    );
    let plan = explicit_command_deterministic_plan_result(
        &state,
        "run explicit command sequence and synthesize answer",
        Some(&route),
        &loop_state,
        request,
        None,
    )
    .expect("command summary should produce observation plus synthesis plan");

    assert_eq!(plan.steps.len(), 4);
    assert_eq!(plan.steps[0].skill, "run_cmd");
    assert_eq!(
        plan.steps[0].args.get("command").and_then(Value::as_str),
        Some("whoami")
    );
    assert_eq!(
        plan.steps[0].args.get(CLAWD_LITERAL_COMMAND_ARG),
        Some(&json!(true))
    );
    assert_eq!(
        plan.steps[0]
            .args
            .get(super::super::super::CLAWD_CONTINUE_ON_ERROR_ARG),
        Some(&json!(true))
    );
    assert_eq!(
        plan.steps[1].args.get("command").and_then(Value::as_str),
        Some("pwd")
    );
    assert_eq!(
        plan.steps[1].args.get(CLAWD_LITERAL_COMMAND_ARG),
        Some(&json!(true))
    );
    assert_eq!(
        plan.steps[1]
            .args
            .get(super::super::super::CLAWD_CONTINUE_ON_ERROR_ARG),
        Some(&json!(true))
    );
    assert_eq!(plan.steps[2].action_type, "synthesize_answer");
    assert_eq!(
        plan.steps[2].args,
        json!({"evidence_refs": ["step_1", "step_2"]})
    );
    assert_eq!(plan.steps[3].action_type, "respond");
    assert_eq!(
        plan.steps[3].args.get("content").and_then(Value::as_str),
        Some("{{last_output}}")
    );
}

#[test]
fn command_output_summary_prefixed_unknown_second_command_gets_multi_step_plan() {
    let mut state = test_state_with_enabled_skills(&["run_cmd"]);
    state.policy.command_intent.execute_prefixes = vec!["执行".to_string()];
    state.policy.command_intent.standalone_commands = vec!["pwd".to_string()];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::CommandOutputSummary;
    route.output_contract.requires_content_evidence = true;
    let loop_state = LoopState::new(1);
    let request =
        "先执行 pwd，再执行 definitely_missing_command_rustclaw_67890，然后总结成功和失败";

    let plan = explicit_command_deterministic_plan_result(
        &state,
        "run command sequence and summarize success/failure",
        Some(&route),
        &loop_state,
        request,
        None,
    )
    .expect("command summary should keep both literal command observations");

    assert_eq!(plan.steps.len(), 4);
    assert_eq!(
        plan.steps[0].args.get("command").and_then(Value::as_str),
        Some("pwd")
    );
    assert_eq!(
        plan.steps[1].args.get("command").and_then(Value::as_str),
        Some("definitely_missing_command_rustclaw_67890")
    );
    for step in plan.steps.iter().take(2) {
        assert_eq!(step.skill, "run_cmd");
        assert_eq!(step.args.get(CLAWD_LITERAL_COMMAND_ARG), Some(&json!(true)));
        assert_eq!(
            step.args.get(CLAWD_CONTINUE_ON_ERROR_ARG),
            Some(&json!(true))
        );
    }
    assert_eq!(
        plan.steps[2].args,
        json!({"evidence_refs": ["step_1", "step_2"]})
    );
    assert_eq!(
        plan.steps[3].args.get("content").and_then(Value::as_str),
        Some("{{last_output}}")
    );
}

#[test]
fn leading_shellish_command_sequence_uses_path_commands() {
    let root = TempDirGuard::new("leading_shellish_command_sequence");
    for command in ["pwd", "whoami", "hostname"] {
        fs::write(root.path.join(command), "").expect("write command marker");
    }

    assert_eq!(
        super::super::leading_shellish_command_sequence_segment_with_path_env(
            "pwd whoami hostname 三个结果每个一行",
            Some(root.path.as_os_str()),
        )
        .as_deref(),
        Some("pwd; whoami; hostname")
    );
}

#[test]
fn leading_shellish_command_sequence_gets_deterministic_run_cmd_plan() {
    if super::super::leading_shellish_command_sequence_segment(
        "pwd whoami hostname 三个结果每个一行",
    )
    .as_deref()
        != Some("pwd; whoami; hostname")
    {
        return;
    }
    let state = test_state_with_enabled_skills(&["run_cmd"]);
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route.output_contract.requires_content_evidence = true;
    let loop_state = LoopState::new(1);
    let request = "pwd whoami hostname 三个结果每个一行 不要总结";

    let plan = explicit_command_deterministic_plan_result(
        &state,
        "run explicit command sequence",
        Some(&route),
        &loop_state,
        request,
        None,
    )
    .expect("leading command sequence should produce run_cmd plan");

    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].skill, "run_cmd");
    assert_eq!(
        plan.steps[0].args.get("command").and_then(Value::as_str),
        Some("pwd; whoami; hostname")
    );
}

#[test]
fn leading_shellish_command_sequence_rejects_plain_status_words() {
    let root = TempDirGuard::new("leading_shellish_command_sequence_reject_status");
    for command in ["pwd", "whoami", "hostname"] {
        fs::write(root.path.join(command), "").expect("write command marker");
    }

    assert!(
        super::super::leading_shellish_command_sequence_segment_with_path_env(
            "show status",
            Some(root.path.as_os_str()),
        )
        .is_none()
    );
}

#[test]
fn leading_shellish_command_sequence_rejects_command_with_argument_shape() {
    let root = TempDirGuard::new("leading_shellish_command_sequence_reject_arg");
    fs::write(root.path.join("ls"), "").expect("write command marker");

    assert!(
        super::super::leading_shellish_command_sequence_segment_with_path_env(
            "ls scripts 结果每行一个",
            Some(root.path.as_os_str()),
        )
        .is_none()
    );
}

#[test]
fn explicit_prefixed_shellish_code_span_keeps_single_step_fast_path() {
    let mut state = test_state_with_enabled_skills(&["run_cmd"]);
    state.policy.command_intent.execute_prefixes = vec!["run ".to_string()];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route.output_contract.requires_content_evidence = true;
    let loop_state = LoopState::new(1);
    let request = "Run `pwd && ls Cargo.toml`.";

    let plan = explicit_command_deterministic_plan_result(
        &state,
        "run explicit shell code span",
        Some(&route),
        &loop_state,
        request,
        None,
    )
    .expect("shellish code span should keep deterministic plan");

    assert_eq!(plan.steps.len(), 1);
    assert_eq!(
        plan.steps[0].args.get("command").and_then(Value::as_str),
        Some("pwd && ls Cargo.toml")
    );
}

#[test]
fn existence_with_path_filename_deterministic_plan_uses_name_search() {
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "start-all-bin.sh".to_string();
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let plan = existence_with_path_locator_deterministic_plan_result(
        "find the file in the current repository",
        Some(&route),
        &loop_state,
        None,
        "find start-all-bin.sh in the current repository",
    )
    .expect("existence-with-path filename route should use a bounded search");

    assert_eq!(plan.plan_kind, PlanKind::Single);
    assert_eq!(plan.steps.len(), 1);
    match &plan.steps[0].to_agent_action() {
        Some(AgentAction::CallTool { tool, args }) => {
            assert_eq!(tool, "fs_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("find_entries")
            );
            assert_eq!(
                args.get("pattern").and_then(Value::as_str),
                Some("start-all-bin.sh")
            );
        }
        other => panic!("expected fs_basic find_entries action, got {other:?}"),
    }
}

#[test]
fn existence_with_path_directory_locator_uses_child_selector_search() {
    let root = TempDirGuard::new("existence_directory_child_selector");
    let locator = root.path.join("locator_smart/fuzzy_top3");
    fs::create_dir_all(&locator).expect("create locator dir");
    fs::write(locator.join("abcd_report.md"), "").expect("write report");
    fs::write(locator.join("my_abcd.txt"), "").expect("write text");
    fs::write(locator.join("x_abcd_log.txt"), "").expect("write log");

    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.resolved_intent =
        "在目录 locator_smart/fuzzy_top3 中查找名称为 abcd 的文件或目录".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = locator.display().to_string();
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let plan = existence_with_path_locator_deterministic_plan_result(
        "find a child selector under an existing directory",
        Some(&route),
        &loop_state,
        Some(&locator.display().to_string()),
        "去 locator_smart/fuzzy_top3 找 abcd",
    )
    .expect("existing directory route should search for the structural child selector");

    assert_eq!(plan.plan_kind, PlanKind::Single);
    assert_eq!(plan.steps.len(), 1);
    match &plan.steps[0].to_agent_action() {
        Some(AgentAction::CallTool { tool, args }) => {
            assert_eq!(tool, "fs_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("find_entries")
            );
            assert_eq!(args.get("pattern").and_then(Value::as_str), Some("abcd"));
            assert_eq!(args.get("target_kind").and_then(Value::as_str), Some("any"));
        }
        other => panic!("expected fs_basic find_entries action, got {other:?}"),
    }
}

#[test]
fn existence_with_path_multi_file_targets_deterministic_plan_uses_path_batch_facts() {
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.resolved_intent =
        "检查 README.md、AGENTS.md、Cargo.toml 是否都存在，只用一行回答每个文件的存在状态"
            .to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = "/home/guagua/rustclaw".to_string();
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let plan = existence_with_path_locator_deterministic_plan_result(
        "check several explicit files",
        Some(&route),
        &loop_state,
        Some("/home/guagua/rustclaw"),
        "检查 README.md、AGENTS.md、Cargo.toml 是否都存在，只用一行回答每个文件的存在状态",
    )
    .expect("multi-file existence route should use path facts");

    assert_eq!(plan.plan_kind, PlanKind::Single);
    assert_eq!(plan.steps.len(), 1);
    match &plan.steps[0].to_agent_action() {
        Some(AgentAction::CallTool { tool, args }) => {
            assert_eq!(tool, "fs_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("stat_paths")
            );
            assert_eq!(
                args.get("paths"),
                Some(&json!(["README.md", "AGENTS.md", "Cargo.toml"]))
            );
        }
        other => panic!("expected fs_basic stat_paths action, got {other:?}"),
    }
}

#[test]
fn existence_with_path_multi_file_targets_preserve_relative_path_segments() {
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.resolved_intent = "Check existence and type of two fixture paths".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = "/home/guagua/rustclaw".to_string();
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;
    let user_text = "Inspecte ces chemins: scripts/nl_tests/fixtures/device_local/package.json et scripts/nl_tests/fixtures/device_local/nope.json; indique existence et type.";

    let plan = existence_with_path_locator_deterministic_plan_result(
        "check several explicit relative fixture paths",
        Some(&route),
        &loop_state,
        Some("/home/guagua/rustclaw"),
        user_text,
    )
    .expect("multi-path existence route should preserve explicit relative paths");

    assert_eq!(plan.plan_kind, PlanKind::Single);
    assert_eq!(plan.steps.len(), 1);
    match &plan.steps[0].to_agent_action() {
        Some(AgentAction::CallTool { tool, args }) => {
            assert_eq!(tool, "fs_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("stat_paths")
            );
            assert_eq!(
                args.get("paths"),
                Some(&json!([
                    "scripts/nl_tests/fixtures/device_local/package.json",
                    "scripts/nl_tests/fixtures/device_local/nope.json"
                ]))
            );
        }
        other => panic!("expected fs_basic stat_paths action, got {other:?}"),
    }
}

#[test]
fn existence_with_path_current_workspace_single_file_target_uses_path_batch_facts() {
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.resolved_intent =
        "Check if README.md exists in the current directory and answer with the path".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = "/home/guagua/rustclaw".to_string();
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let plan = existence_with_path_locator_deterministic_plan_result(
        "check one explicit file in current workspace",
        Some(&route),
        &loop_state,
        Some("/home/guagua/rustclaw"),
        "Check if README.md exists in the current directory and answer with the path",
    )
    .expect("single-file current-workspace existence route should use path facts");

    assert_eq!(plan.plan_kind, PlanKind::Single);
    assert_eq!(plan.steps.len(), 1);
    match &plan.steps[0].to_agent_action() {
        Some(AgentAction::CallTool { tool, args }) => {
            assert_eq!(tool, "fs_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("stat_paths")
            );
            assert_eq!(args.get("paths"), Some(&json!(["README.md"])));
        }
        other => panic!("expected fs_basic stat_paths action, got {other:?}"),
    }
}

#[test]
fn existence_with_path_current_workspace_service_file_target_uses_path_batch_facts() {
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.resolved_intent =
        "Check whether rustclaw.service exists in the current repository and include the path"
            .to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = "/home/guagua/rustclaw".to_string();
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let plan = existence_with_path_locator_deterministic_plan_result(
        "check one service file in current workspace",
        Some(&route),
        &loop_state,
        Some("/home/guagua/rustclaw"),
        "检查仓库里有没有 rustclaw.service，只回答有或没有，并给出路径",
    )
    .expect("single service-file current-workspace existence route should use path facts");

    assert_eq!(plan.plan_kind, PlanKind::Single);
    assert_eq!(plan.steps.len(), 1);
    match &plan.steps[0].to_agent_action() {
        Some(AgentAction::CallTool { tool, args }) => {
            assert_eq!(tool, "fs_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("stat_paths")
            );
            assert_eq!(args.get("paths"), Some(&json!(["rustclaw.service"])));
        }
        other => panic!("expected fs_basic stat_paths action, got {other:?}"),
    }
}

#[test]
fn existence_with_path_path_deterministic_plan_uses_path_facts() {
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "Cargo.lock".to_string();
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let plan = existence_with_path_locator_deterministic_plan_result(
        "check exact path existence",
        Some(&route),
        &loop_state,
        Some("/tmp/Cargo.lock"),
        "check exact path Cargo.lock existence",
    )
    .expect("existence-with-path path route should use path facts");

    assert_eq!(plan.plan_kind, PlanKind::Single);
    assert_eq!(plan.steps.len(), 1);
    match &plan.steps[0].to_agent_action() {
        Some(AgentAction::CallTool { tool, args }) => {
            assert_eq!(tool, "fs_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("stat_paths")
            );
            assert_eq!(args.get("paths"), Some(&json!(["/tmp/Cargo.lock"])));
        }
        other => panic!("expected fs_basic stat_paths action, got {other:?}"),
    }
}

#[test]
fn existence_with_path_retry_read_text_range_is_preserved_when_verifier_requests_excerpt() {
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "README.md".to_string();
    route.output_contract.delivery_required = false;
    route.output_contract.requires_content_evidence = true;
    route.resolved_intent = "Check README.md existence and return a bounded slice.".to_string();
    let mut loop_state = LoopState::default();
    loop_state.round_no = 2;
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "read_text_range",
            "path": "/home/guagua/rustclaw/README.md",
            "mode": "head",
            "n": 5
        }),
    }];

    let normalized = normalize_planned_actions(
        &test_state(),
        Some(&route),
        &loop_state,
        "retry with content_excerpt after verifier requested it",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallTool { tool, args } | AgentAction::CallSkill { skill: tool, args } => {
            assert_eq!(tool, "fs_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("read_text_range")
            );
            assert_eq!(args.get("mode").and_then(Value::as_str), Some("head"));
            assert_eq!(args.get("n").and_then(Value::as_u64), Some(5));
        }
        other => panic!("expected fs_basic read_text_range action, got {other:?}"),
    }
}

#[test]
fn archive_entry_existence_uses_archive_list_instead_of_archive_stat() {
    let state = test_state_with_enabled_skills(&["archive_basic"]);
    let archive = "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip";
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.resolved_intent =
        format!("Check whether archive member nested/config.ini is present in {archive}.");
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = archive.to_string();
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let stat_plan = existence_with_path_locator_deterministic_plan_result(
        "check archive member existence",
        Some(&route),
        &loop_state,
        Some(archive),
        "nested/config.ini in scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip",
    );
    assert!(
        stat_plan.is_none(),
        "archive member checks must not be answered by statting only the archive file"
    );

    let plan = archive_list_auto_locator_deterministic_plan_result(
        "check archive member existence",
        &state,
        Some(&route),
        &loop_state,
        Some(archive),
    )
    .expect("archive member existence should inspect archive entries");

    assert_eq!(plan.steps.len(), 3);
    match &plan.steps[0].to_agent_action() {
        Some(AgentAction::CallSkill { skill, args }) => {
            assert_eq!(skill, "archive_basic");
            assert_eq!(args.get("action").and_then(Value::as_str), Some("list"));
            assert_eq!(args.get("archive").and_then(Value::as_str), Some(archive));
        }
        other => panic!("expected archive_basic list action, got {other:?}"),
    }
}

#[test]
fn archive_entry_existence_scalar_shape_uses_archive_list() {
    let state = test_state_with_enabled_skills(&["archive_basic"]);
    let archive = "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip";
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.resolved_intent =
        format!("Check whether archive member notes.txt is present in {archive}.");
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = archive.to_string();
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let stat_plan = existence_with_path_locator_deterministic_plan_result(
            "check archive member scalar existence",
            Some(&route),
            &loop_state,
            Some(archive),
            "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip 에 notes.txt 가 있는지만 말해. 압축 풀지 마.",
        );
    assert!(
        stat_plan.is_none(),
        "archive member scalar checks must not stat only the archive file"
    );

    let plan = archive_list_auto_locator_deterministic_plan_result(
        "check archive member scalar existence",
        &state,
        Some(&route),
        &loop_state,
        Some(archive),
    )
    .expect("archive member scalar existence should inspect archive entries");

    match &plan.steps[0].to_agent_action() {
        Some(AgentAction::CallSkill { skill, args }) => {
            assert_eq!(skill, "archive_basic");
            assert_eq!(args.get("action").and_then(Value::as_str), Some("list"));
            assert_eq!(args.get("archive").and_then(Value::as_str), Some(archive));
        }
        other => panic!("expected archive_basic list action, got {other:?}"),
    }
}

#[test]
fn archive_file_existence_without_member_target_still_stats_archive() {
    let archive = "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip";
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.resolved_intent = format!("Check whether {archive} exists.");
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = archive.to_string();
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let plan = existence_with_path_locator_deterministic_plan_result(
        "check archive file existence",
        Some(&route),
        &loop_state,
        Some(archive),
        "Check whether scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip exists.",
    )
    .expect("plain archive file existence should use path facts");

    assert_eq!(plan.steps.len(), 1);
    match &plan.steps[0].to_agent_action() {
        Some(AgentAction::CallTool { tool, args }) => {
            assert_eq!(tool, "fs_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("stat_paths")
            );
            assert_eq!(args.get("paths"), Some(&json!([archive])));
        }
        other => panic!("expected fs_basic stat_paths action, got {other:?}"),
    }
}

#[test]
fn existence_with_path_directory_locator_with_file_target_uses_find_path() {
    let root = TempDirGuard::new("existence_dir_locator_file_target");
    fs::create_dir_all(root.path.join("case_only")).expect("mkdir");
    let directory = root.path.join("case_only");
    let directory_path = directory.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.resolved_intent =
        "Locate report.md within the specified directory and output only its full path."
            .to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = directory_path.clone();
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let plan = existence_with_path_locator_deterministic_plan_result(
        "find a file inside a resolved directory",
        Some(&route),
        &loop_state,
        Some(&directory_path),
        "Locate report.md within the specified directory and output only its full path.",
    )
    .expect("directory locator with file target should use find_path");

    assert_eq!(plan.plan_kind, PlanKind::Single);
    assert_eq!(plan.steps.len(), 1);
    match &plan.steps[0].to_agent_action() {
        Some(AgentAction::CallTool { tool, args }) => {
            assert_eq!(tool, "fs_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("find_entries")
            );
            assert_eq!(
                args.get("root").and_then(Value::as_str),
                Some(directory_path.as_str())
            );
            assert_eq!(
                args.get("pattern").and_then(Value::as_str),
                Some("report.md")
            );
            assert_eq!(
                args.get("target_kind").and_then(Value::as_str),
                Some("file")
            );
        }
        other => panic!("expected fs_basic find_entries action, got {other:?}"),
    }
}

#[test]
fn existence_with_path_directory_auto_locator_does_not_parse_history_entries_as_targets() {
    let root = TempDirGuard::new("existence_dir_locator_history_entries");
    fs::create_dir_all(root.path.join("configs")).expect("mkdir configs");
    let directory_path = root.path.join("configs").display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Strict,
    );
    route.resolved_intent = "Current task:\n先列出 configs 目录下前 5 个条目名称\n\nMost recent generated output:\nagent_guard.toml\naudio.toml\nbrowser_web_wait_map.json\nchannel_commands.toml\nchannels\n\nNew user instruction:\n看最后一个的基本信息，只回答路径和类型".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = directory_path.clone();
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let plan = existence_with_path_locator_deterministic_plan_result(
        "follow up on the active ordered list",
        Some(&route),
        &loop_state,
        Some(&directory_path),
        "看最后一个的基本信息，只回答路径和类型",
    );

    assert!(
            plan.is_none(),
            "directory auto-locator followups without current-turn locator surface should stay with planner/anchor resolution"
        );
}

#[test]
fn file_paths_current_workspace_deterministic_plan_uses_name_search() {
    let root = TempDirGuard::new("file_paths_deterministic_plan");
    let script = root.path.join("start-all-bin.sh");
    fs::write(&script, "#!/usr/bin/env bash\n").expect("write script");
    let script_path = script.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::FilePaths;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = "start-all-bin.sh".to_string();
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let plan = file_paths_locator_deterministic_plan_result(
        "find a matching file and return its relative path",
        Some(&route),
        &loop_state,
        Some(&script_path),
    )
    .expect("file-path route should use a bounded name search");

    assert_eq!(plan.plan_kind, PlanKind::Single);
    assert_eq!(plan.steps.len(), 1);
    match &plan.steps[0].to_agent_action() {
        Some(AgentAction::CallTool { tool, args }) => {
            assert_eq!(tool, "fs_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("find_entries")
            );
            assert_eq!(
                args.get("pattern").and_then(Value::as_str),
                Some("start-all-bin.sh")
            );
            assert_eq!(
                args.get("target_kind").and_then(Value::as_str),
                Some("file")
            );
        }
        other => panic!("expected fs_basic find_entries action, got {other:?}"),
    }
}

#[test]
fn file_paths_path_like_locator_hint_uses_parent_search_scope() {
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::FilePaths;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = "case_only/report.md".to_string();
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let plan = file_paths_locator_deterministic_plan_result(
        "find path-like locator under its parent scope",
        Some(&route),
        &loop_state,
        None,
    )
    .expect("path-like file locator should preserve parent scope");

    assert_eq!(plan.plan_kind, PlanKind::Single);
    assert_eq!(plan.steps.len(), 1);
    match &plan.steps[0].to_agent_action() {
        Some(AgentAction::CallTool { tool, args }) => {
            assert_eq!(tool, "fs_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("find_entries")
            );
            assert_eq!(args.get("root").and_then(Value::as_str), Some("case_only"));
            assert_eq!(
                args.get("pattern").and_then(Value::as_str),
                Some("report.md")
            );
            assert_eq!(
                args.get("target_kind").and_then(Value::as_str),
                Some("file")
            );
        }
        other => panic!("expected fs_basic find_entries action, got {other:?}"),
    }
}

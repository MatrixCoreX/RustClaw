use super::*;

fn normalize_explicit_planner_actions(
    state: &AppState,
    route: &RouteResult,
    loop_state: &LoopState,
    goal: &str,
    request: &str,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    normalize_planned_actions_with_original(
        state,
        Some(route),
        loop_state,
        goal,
        Some(request),
        Some(state.skill_rt.workspace_root.to_string_lossy().as_ref()),
        actions,
    )
}

fn planner_run_cmd_action(
    state: &AppState,
    request: &str,
    command: &str,
    continue_on_error: bool,
) -> AgentAction {
    let mut args = json!({
        "command": command,
        "request_text": request,
        "cwd": state.skill_rt.workspace_root.display().to_string(),
    });
    if continue_on_error {
        args[CLAWD_CONTINUE_ON_ERROR_ARG] = Value::Bool(true);
    }
    AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args,
    }
}

fn assert_run_cmd_action<'a>(action: &'a AgentAction, command: &str) -> &'a Value {
    let args = super::super::action_args(action).expect("run_cmd args");
    match action {
        AgentAction::CallTool { tool, .. } | AgentAction::CallSkill { skill: tool, .. } => {
            assert_eq!(tool, "run_cmd");
        }
        other => panic!("expected run_cmd action, got {other:?}"),
    }
    assert_eq!(args.get("command").and_then(Value::as_str), Some(command));
    assert_eq!(args.get(CLAWD_LITERAL_COMMAND_ARG), Some(&json!(true)));
    args
}

fn assert_synthesize_action(action: &AgentAction, evidence_refs: &[&str]) {
    assert!(matches!(
        action,
        AgentAction::SynthesizeAnswer { evidence_refs: refs }
            if refs == &evidence_refs.iter().map(|value| value.to_string()).collect::<Vec<_>>()
    ));
}

fn assert_last_output_respond(action: &AgentAction) {
    assert!(matches!(
        action,
        AgentAction::Respond { content } if content == "{{last_output}}"
    ));
}

fn assert_empty_planner_actions_stay_empty(
    state: &AppState,
    route: &RouteResult,
    loop_state: &LoopState,
    goal: &str,
    request: &str,
) {
    let normalized =
        normalize_explicit_planner_actions(state, route, loop_state, goal, request, vec![]);
    assert!(
        normalized.is_empty(),
        "runtime must not inject an explicit-command plan before the planner: {normalized:?}"
    );
}

#[test]
fn execution_failed_step_prefixed_bare_sequence_gets_multi_run_cmd_plan() {
    let mut state = test_state_with_enabled_skills(&["run_cmd"]);
    state.policy.command_intent.execute_prefixes = vec!["执行".to_string()];
    state.policy.command_intent.standalone_commands = vec!["pwd".to_string()];
    let mut route = route_result(
        crate::AskMode::act_with_chat_finalizer(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ExecutionFailedStep;
    route.output_contract.requires_content_evidence = true;
    let loop_state = LoopState::new(1);
    let request =
        "先执行 pwd，再执行 definitely_missing_command_rustclaw_67890，然后总结成功和失败";

    let normalized = normalize_explicit_planner_actions(
        &state,
        &route,
        &loop_state,
        "run command sequence and summarize success/failure",
        request,
        vec![
            planner_run_cmd_action(&state, request, "pwd", true),
            planner_run_cmd_action(
                &state,
                request,
                "definitely_missing_command_rustclaw_67890",
                true,
            ),
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec!["step_1".to_string(), "step_2".to_string()],
            },
            AgentAction::Respond {
                content: "{{last_output}}".to_string(),
            },
        ],
    );

    assert_eq!(normalized.len(), 4);
    let args = assert_run_cmd_action(&normalized[0], "pwd");
    assert_eq!(args.get(CLAWD_CONTINUE_ON_ERROR_ARG), Some(&json!(true)));
    let args = assert_run_cmd_action(&normalized[1], "definitely_missing_command_rustclaw_67890");
    assert_eq!(args.get(CLAWD_CONTINUE_ON_ERROR_ARG), Some(&json!(true)));
    assert_synthesize_action(&normalized[2], &["step_1", "step_2"]);
    assert_last_output_respond(&normalized[3]);
}

#[test]
fn execution_failed_step_prefixed_echo_sequence_counts_as_explicit_command_request() {
    let mut state = test_state_with_enabled_skills(&["run_cmd"]);
    state.policy.command_intent.execute_prefixes = vec!["执行".to_string()];
    let mut route = route_result(
        crate::AskMode::act_with_chat_finalizer(),
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
    let normalized = normalize_explicit_planner_actions(
        &state,
        &route,
        &loop_state,
        "run command sequence and continue after failure",
        request,
        vec![
            planner_run_cmd_action(&state, request, "echo BEFORE_BREAK", false),
            planner_run_cmd_action(
                &state,
                request,
                "definitely_missing_command_rustclaw_67890",
                false,
            ),
            planner_run_cmd_action(&state, request, "echo AFTER_BREAK_67890", false),
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec![
                    "step_1".to_string(),
                    "step_2".to_string(),
                    "step_3".to_string(),
                ],
            },
            AgentAction::Respond {
                content: "{{last_output}}".to_string(),
            },
        ],
    );

    assert_eq!(normalized.len(), 5);
    for (action, command) in normalized.iter().take(3).zip([
        "echo BEFORE_BREAK",
        "definitely_missing_command_rustclaw_67890",
        "echo AFTER_BREAK_67890",
    ]) {
        let args = assert_run_cmd_action(action, command);
        assert_eq!(
            args.get(CLAWD_CONTINUE_ON_ERROR_ARG),
            None,
            "planner did not request continue-on-error for this command"
        );
    }
    assert_synthesize_action(&normalized[3], &["step_1", "step_2", "step_3"]);
    assert_last_output_respond(&normalized[4]);
}

#[test]
fn conditional_step_update_limits_current_explicit_command_plan_to_pre_update_steps() {
    let state = test_state_with_enabled_skills(&["run_cmd"]);
    let mut route = route_result(
        crate::AskMode::act_with_chat_finalizer(),
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

    let normalized = normalize_explicit_planner_actions(
        &state,
        &route,
        &loop_state,
        "run failed command prefix and remember conditional continuation",
        request,
        vec![
            planner_run_cmd_action(&state, request, "echo BEFORE_CHANGE_EN", false),
            planner_run_cmd_action(
                &state,
                request,
                "definitely_missing_command_rustclaw_change_24683",
                false,
            ),
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec!["step_1".to_string(), "step_2".to_string()],
            },
            AgentAction::Respond {
                content: "{{last_output}}".to_string(),
            },
        ],
    );

    assert_eq!(normalized.len(), 4);
    for (action, command) in normalized.iter().take(2).zip([
        "echo BEFORE_CHANGE_EN",
        "definitely_missing_command_rustclaw_change_24683",
    ]) {
        let args = assert_run_cmd_action(action, command);
        assert_eq!(args.get(CLAWD_CONTINUE_ON_ERROR_ARG), None);
    }
    assert_synthesize_action(&normalized[2], &["step_1", "step_2"]);
    assert_last_output_respond(&normalized[3]);
    let executed = normalized
        .iter()
        .filter_map(|action| {
            super::super::action_args(action).and_then(|args| args.get("command")?.as_str())
        })
        .collect::<Vec<_>>();
    assert!(!executed.contains(&"echo AFTER_CHANGE_OLD_EN"));
    assert!(!executed.contains(&"echo AFTER_CHANGE_NEW_EN"));
    assert_eq!(
        super::super::conditional_step_update_immediate_command_count(Some(&turn_analysis)),
        Some(2)
    );
}

#[test]
fn multi_explicit_run_cmd_plan_marks_literal_commands_without_continue_on_error() {
    let mut route = route_result(
        crate::AskMode::act_plain(),
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
        Some(
            "先执行 echo BEFORE_CHANGE，再执行 definitely_missing_command_rustclaw_change_24682，再执行 echo AFTER_CHANGE_OLD；如果稍后继续则替换最后一步。",
        ),
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
        crate::AskMode::act_plain(),
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
    assert_empty_planner_actions_stay_empty(
        &state,
        &route,
        &loop_state,
        "run compound explicit commands",
        request,
    );
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
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route.output_contract.requires_content_evidence = true;
    let loop_state = LoopState::new(1);
    let request = "Run pwd,";

    let normalized = normalize_explicit_planner_actions(
        &state,
        &route,
        &loop_state,
        "run explicit command",
        request,
        vec![planner_run_cmd_action(&state, request, "pwd", false)],
    );

    assert_eq!(normalized.len(), 1);
    assert_run_cmd_action(&normalized[0], "pwd");
}

#[test]
fn explicit_configured_command_inside_clause_is_detected() {
    let mut state = test_state_with_enabled_skills(&["run_cmd"]);
    state.policy.command_intent.execute_prefixes = vec!["请执行".to_string(), "执行".to_string()];
    let mut route = route_result(
        crate::AskMode::act_plain(),
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
    let normalized = normalize_explicit_planner_actions(
        &state,
        &route,
        &loop_state,
        "run explicit command",
        request,
        vec![planner_run_cmd_action(&state, request, "pwd", false)],
    );

    assert_eq!(normalized.len(), 1);
    assert_run_cmd_action(&normalized[0], "pwd");
}

#[test]
fn embedded_standalone_command_with_structural_args_keeps_single_step_fast_path() {
    let mut state = test_state_with_enabled_skills(&["run_cmd"]);
    state.policy.command_intent.execute_prefixes = vec!["执行".to_string()];
    state.policy.command_intent.standalone_commands = vec!["pwd".to_string()];
    let mut route = route_result(
        crate::AskMode::act_plain(),
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
    let normalized = normalize_explicit_planner_actions(
        &state,
        &route,
        &loop_state,
        "run explicit command",
        request,
        vec![planner_run_cmd_action(&state, request, "pwd -P", false)],
    );

    assert_eq!(normalized.len(), 1);
    assert_run_cmd_action(&normalized[0], "pwd -P");
}

#[test]
fn embedded_standalone_command_sequence_uses_configured_command_tokens() {
    let mut state = test_state_with_enabled_skills(&["run_cmd"]);
    state.policy.command_intent.execute_prefixes = vec!["执行".to_string()];
    state.policy.command_intent.standalone_commands = vec!["pwd".to_string(), "whoami".to_string()];
    let mut route = route_result(
        crate::AskMode::act_plain(),
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
    let normalized = normalize_explicit_planner_actions(
        &state,
        &route,
        &loop_state,
        "run explicit command sequence",
        request,
        vec![
            planner_run_cmd_action(&state, request, "pwd", false),
            planner_run_cmd_action(&state, request, "whoami", false),
        ],
    );

    assert_eq!(normalized.len(), 2);
    assert_run_cmd_action(&normalized[0], "pwd");
    assert_run_cmd_action(&normalized[1], "whoami");
}

#[test]
fn prefixed_single_command_with_format_tail_is_single_step_safe() {
    let mut state = test_state_with_enabled_skills(&["run_cmd"]);
    state.policy.command_intent.execute_prefixes = vec!["执行".to_string()];
    state.policy.command_intent.standalone_commands = vec!["pwd".to_string(), "whoami".to_string()];

    assert_eq!(
        super::super::explicit_command_single_step_segment(
            &state.policy.command_intent,
            "执行 pwd，只输出当前工作目录的绝对路径"
        )
        .as_deref(),
        Some("pwd")
    );
}

#[test]
fn prefixed_single_command_with_second_command_tail_is_not_single_step_safe() {
    let mut state = test_state_with_enabled_skills(&["run_cmd"]);
    state.policy.command_intent.execute_prefixes = vec!["执行".to_string()];
    state.policy.command_intent.standalone_commands = vec!["pwd".to_string(), "whoami".to_string()];

    assert_eq!(
        super::super::explicit_command_single_step_segment(
            &state.policy.command_intent,
            "执行 pwd，再执行 whoami，然后输出结果"
        ),
        None
    );
}

#[test]
fn command_output_summary_explicit_command_plan_synthesizes_configured_sequence() {
    let mut state = test_state_with_enabled_skills(&["run_cmd"]);
    state.policy.command_intent.execute_prefixes = vec!["执行".to_string()];
    state.policy.command_intent.standalone_commands = vec!["pwd".to_string(), "whoami".to_string()];
    let mut route = route_result(
        crate::AskMode::act_plain(),
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
    let normalized = normalize_explicit_planner_actions(
        &state,
        &route,
        &loop_state,
        "run explicit command sequence and synthesize answer",
        request,
        vec![
            planner_run_cmd_action(&state, request, "whoami", true),
            planner_run_cmd_action(&state, request, "pwd", true),
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec!["step_1".to_string(), "step_2".to_string()],
            },
            AgentAction::Respond {
                content: "{{last_output}}".to_string(),
            },
        ],
    );

    assert_eq!(normalized.len(), 4);
    let args = assert_run_cmd_action(&normalized[0], "whoami");
    assert_eq!(args.get(CLAWD_CONTINUE_ON_ERROR_ARG), Some(&json!(true)));
    let args = assert_run_cmd_action(&normalized[1], "pwd");
    assert_eq!(args.get(CLAWD_CONTINUE_ON_ERROR_ARG), Some(&json!(true)));
    assert_synthesize_action(&normalized[2], &["step_1", "step_2"]);
    assert_last_output_respond(&normalized[3]);
}

#[test]
fn command_output_summary_embedded_code_span_after_run_prefix_uses_literal_command() {
    let mut state = test_state_with_enabled_skills(&["run_cmd"]);
    state.policy.command_intent.execute_prefixes = vec!["run ".to_string()];
    state.policy.command_intent.standalone_commands = vec!["pwd".to_string()];
    let mut route = route_result(
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::CommandOutputSummary;
    route.output_contract.requires_content_evidence = true;
    let loop_state = LoopState::new(1);
    let request = "Run the explicit shell command `pwd`, then inspect local listening ports or processes related to clawd; answer with the working directory and whether a clawd-related process or port is visible.";

    assert_eq!(
        super::super::explicit_command_segment(&state.policy.command_intent, request).as_deref(),
        Some("pwd")
    );
    assert_eq!(
        super::super::explicit_command_single_step_segment(&state.policy.command_intent, request)
            .as_deref(),
        Some("pwd")
    );
    let normalized = normalize_explicit_planner_actions(
        &state,
        &route,
        &loop_state,
        "run pwd and summarize related process or port evidence",
        request,
        vec![
            planner_run_cmd_action(&state, request, "pwd", false),
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec!["last_output".to_string()],
            },
            AgentAction::Respond {
                content: "{{last_output}}".to_string(),
            },
        ],
    );

    assert_eq!(normalized.len(), 3);
    assert_run_cmd_action(&normalized[0], "pwd");
    assert_synthesize_action(&normalized[1], &["last_output"]);
    assert_last_output_respond(&normalized[2]);
}

#[test]
fn command_output_summary_prefixed_unknown_second_command_gets_multi_step_plan() {
    let mut state = test_state_with_enabled_skills(&["run_cmd"]);
    state.policy.command_intent.execute_prefixes = vec!["执行".to_string()];
    state.policy.command_intent.standalone_commands = vec!["pwd".to_string()];
    let mut route = route_result(
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::CommandOutputSummary;
    route.output_contract.requires_content_evidence = true;
    let loop_state = LoopState::new(1);
    let request =
        "先执行 pwd，再执行 definitely_missing_command_rustclaw_67890，然后总结成功和失败";

    let normalized = normalize_explicit_planner_actions(
        &state,
        &route,
        &loop_state,
        "run command sequence and summarize success/failure",
        request,
        vec![
            planner_run_cmd_action(&state, request, "pwd", true),
            planner_run_cmd_action(
                &state,
                request,
                "definitely_missing_command_rustclaw_67890",
                true,
            ),
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec!["step_1".to_string(), "step_2".to_string()],
            },
            AgentAction::Respond {
                content: "{{last_output}}".to_string(),
            },
        ],
    );

    assert_eq!(normalized.len(), 4);
    let args = assert_run_cmd_action(&normalized[0], "pwd");
    assert_eq!(args.get(CLAWD_CONTINUE_ON_ERROR_ARG), Some(&json!(true)));
    let args = assert_run_cmd_action(&normalized[1], "definitely_missing_command_rustclaw_67890");
    assert_eq!(args.get(CLAWD_CONTINUE_ON_ERROR_ARG), Some(&json!(true)));
    assert_synthesize_action(&normalized[2], &["step_1", "step_2"]);
    assert_last_output_respond(&normalized[3]);
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
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route.output_contract.requires_content_evidence = true;
    let loop_state = LoopState::new(1);
    let request = "pwd whoami hostname 三个结果每个一行 不要总结";

    let normalized = normalize_explicit_planner_actions(
        &state,
        &route,
        &loop_state,
        "run explicit command sequence",
        request,
        vec![
            planner_run_cmd_action(&state, request, "pwd", false),
            planner_run_cmd_action(&state, request, "whoami", false),
            planner_run_cmd_action(&state, request, "hostname", false),
        ],
    );

    assert_eq!(normalized.len(), 3);
    assert_run_cmd_action(&normalized[0], "pwd");
    assert_run_cmd_action(&normalized[1], "whoami");
    assert_run_cmd_action(&normalized[2], "hostname");
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
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route.output_contract.requires_content_evidence = true;
    let loop_state = LoopState::new(1);
    let request = "Run `pwd && ls Cargo.toml`.";

    let normalized = normalize_explicit_planner_actions(
        &state,
        &route,
        &loop_state,
        "run explicit shell code span",
        request,
        vec![planner_run_cmd_action(
            &state,
            request,
            "pwd && ls Cargo.toml",
            false,
        )],
    );

    assert_eq!(normalized.len(), 1);
    assert_run_cmd_action(&normalized[0], "pwd && ls Cargo.toml");
}

#[test]
fn existence_with_path_filename_deterministic_plan_uses_name_search() {
    let mut route = route_result(
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "start-all-bin.sh".to_string();
    route.output_contract.delivery_required = false;
    route.route_reason = "capability_ref=filesystem.find_entries".to_string();
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let args = assert_planner_supplied_tool_call_preserved(
        &test_state(),
        &route,
        &loop_state,
        "find the file in the current repository",
        Some("find start-all-bin.sh in the current repository"),
        None,
        "fs_basic",
        "find_entries",
        json!({
            "action": "find_entries",
            "pattern": "start-all-bin.sh",
            "target_kind": "any",
            "max_results": 50,
        }),
    );

    assert_eq!(
        args.get("pattern").and_then(Value::as_str),
        Some("start-all-bin.sh")
    );
}

#[test]
fn existence_with_path_directory_locator_uses_child_selector_search() {
    let root = TempDirGuard::new("existence_directory_child_selector");
    let locator = root.path.join("locator_smart/fuzzy_top3");
    fs::create_dir_all(&locator).expect("create locator dir");
    fs::write(locator.join("abcd_report.md"), "").expect("write report");
    fs::write(locator.join("my_abcd.txt"), "").expect("write text");
    fs::write(locator.join("x_abcd_log.txt"), "").expect("write log");
    let locator_path = locator.display().to_string();

    let mut route = route_result(
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.resolved_intent =
        "在目录 locator_smart/fuzzy_top3 中查找名称为 abcd 的文件或目录".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = locator_path.clone();
    route.output_contract.delivery_required = false;
    route.route_reason = "capability_ref=filesystem.find_entries".to_string();
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let args = assert_planner_supplied_tool_call_preserved(
        &test_state(),
        &route,
        &loop_state,
        "find a child selector under an existing directory",
        Some(&locator_path),
        Some(&locator_path),
        "fs_basic",
        "find_entries",
        json!({
            "action": "find_entries",
            "root": locator_path.clone(),
            "pattern": "abcd",
            "target_kind": "any",
        }),
    );

    assert_eq!(args.get("pattern").and_then(Value::as_str), Some("abcd"));
    assert_eq!(args.get("target_kind").and_then(Value::as_str), Some("any"));
}

#[test]
fn existence_with_path_multi_file_targets_deterministic_plan_uses_path_batch_facts() {
    let mut route = route_result(
        crate::AskMode::act_plain(),
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
    route.route_reason = "capability_ref=filesystem.stat_paths".to_string();
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let args = assert_planner_supplied_tool_call_preserved(
        &test_state(),
        &route,
        &loop_state,
        "check several explicit files",
        Some("check several explicit files"),
        Some("/home/guagua/rustclaw"),
        "fs_basic",
        "stat_paths",
        json!({
            "action": "stat_paths",
            "paths": ["README.md", "AGENTS.md", "Cargo.toml"],
        }),
    );

    assert_eq!(
        args.get("paths"),
        Some(&json!(["README.md", "AGENTS.md", "Cargo.toml"]))
    );
}

#[test]
fn existence_with_path_multi_file_targets_preserve_relative_path_segments() {
    let mut route = route_result(
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.resolved_intent = "Check existence and type of two fixture paths".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = "/home/guagua/rustclaw".to_string();
    route.output_contract.delivery_required = false;
    route.route_reason = "capability_ref=filesystem.stat_paths".to_string();
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;
    let user_text = "Inspecte ces chemins: scripts/nl_tests/fixtures/device_local/package.json et scripts/nl_tests/fixtures/device_local/nope.json; indique existence et type.";

    let args = assert_planner_supplied_tool_call_preserved(
        &test_state(),
        &route,
        &loop_state,
        "check several explicit relative fixture paths",
        Some(user_text),
        Some("/home/guagua/rustclaw"),
        "fs_basic",
        "stat_paths",
        json!({
            "action": "stat_paths",
            "paths": [
                "scripts/nl_tests/fixtures/device_local/package.json",
                "scripts/nl_tests/fixtures/device_local/nope.json"
            ],
        }),
    );

    assert_eq!(
        args.get("paths"),
        Some(&json!([
            "scripts/nl_tests/fixtures/device_local/package.json",
            "scripts/nl_tests/fixtures/device_local/nope.json"
        ]))
    );
}

#[test]
fn existence_with_path_current_workspace_single_file_target_uses_path_batch_facts() {
    let mut route = route_result(
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.resolved_intent =
        "Check if README.md exists in the current directory and answer with the path".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = "/home/guagua/rustclaw".to_string();
    route.output_contract.delivery_required = false;
    route.route_reason = "capability_ref=filesystem.stat_paths".to_string();
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let args = assert_planner_supplied_tool_call_preserved(
        &test_state(),
        &route,
        &loop_state,
        "check one explicit file in current workspace",
        Some("Check if README.md exists in the current directory and answer with the path"),
        Some("/home/guagua/rustclaw"),
        "fs_basic",
        "stat_paths",
        json!({
            "action": "stat_paths",
            "paths": ["README.md"],
        }),
    );

    assert_eq!(args.get("paths"), Some(&json!(["README.md"])));
}

#[test]
fn missing_existing_file_delivery_uses_find_name_probe() {
    let mut route = route_result(
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::FileToken,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.locator_hint =
        "definitely_missing_named_file_route_cleanup_001.txt".to_string();
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.wants_file_delivery = true;
    route.route_reason = "capability_ref=filesystem.find_entries".to_string();
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let args = assert_planner_supplied_tool_call_preserved(
        &test_state(),
        &route,
        &loop_state,
        "deliver an existing file if present",
        Some("把 definitely_missing_named_file_route_cleanup_001.txt 发给我"),
        None,
        "fs_basic",
        "find_entries",
        json!({
            "action": "find_entries",
            "root": ".",
            "pattern": "definitely_missing_named_file_route_cleanup_001.txt",
        }),
    );

    assert_eq!(
        args.get("pattern").and_then(Value::as_str),
        Some("definitely_missing_named_file_route_cleanup_001.txt")
    );
    assert_eq!(args.get("root").and_then(Value::as_str), Some("."));
}

#[test]
fn generated_file_delivery_without_state_patch_uses_existing_file_probe() {
    let mut route = route_result(
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::FileToken,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::GeneratedFileDelivery;
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.locator_hint =
        "definitely_missing_named_file_route_cleanup_001.txt".to_string();
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.wants_file_delivery = true;
    route.route_reason = "capability_ref=filesystem.find_entries".to_string();
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let args = assert_planner_supplied_tool_call_preserved(
        &test_state(),
        &route,
        &loop_state,
        "deliver an existing file if present",
        Some("把 definitely_missing_named_file_route_cleanup_001.txt 发给我"),
        None,
        "fs_basic",
        "find_entries",
        json!({
            "action": "find_entries",
            "pattern": "definitely_missing_named_file_route_cleanup_001.txt",
        }),
    );

    assert_eq!(
        args.get("pattern").and_then(Value::as_str),
        Some("definitely_missing_named_file_route_cleanup_001.txt")
    );
}

#[test]
fn existence_with_path_current_workspace_service_file_target_uses_path_batch_facts() {
    let mut route = route_result(
        crate::AskMode::act_plain(),
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
    route.route_reason = "capability_ref=filesystem.stat_paths".to_string();
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let args = assert_planner_supplied_tool_call_preserved(
        &test_state(),
        &route,
        &loop_state,
        "check one service file in current workspace",
        Some("检查仓库里有没有 rustclaw.service，只回答有或没有，并给出路径"),
        Some("/home/guagua/rustclaw"),
        "fs_basic",
        "stat_paths",
        json!({
            "action": "stat_paths",
            "paths": ["rustclaw.service"],
        }),
    );

    assert_eq!(args.get("paths"), Some(&json!(["rustclaw.service"])));
}

#[test]
fn existence_with_path_path_deterministic_plan_uses_path_facts() {
    let mut route = route_result(
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "Cargo.lock".to_string();
    route.output_contract.delivery_required = false;
    route.route_reason = "capability_ref=filesystem.stat_paths".to_string();
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let args = assert_planner_supplied_tool_call_preserved(
        &test_state(),
        &route,
        &loop_state,
        "check exact path existence",
        Some("check exact path Cargo.lock existence"),
        Some("/tmp/Cargo.lock"),
        "fs_basic",
        "stat_paths",
        json!({
            "action": "stat_paths",
            "paths": ["/tmp/Cargo.lock"],
        }),
    );

    assert_eq!(args.get("paths"), Some(&json!(["/tmp/Cargo.lock"])));
}

#[test]
fn existence_with_path_retry_read_text_range_is_preserved_when_verifier_requests_excerpt() {
    let mut route = route_result(
        crate::AskMode::act_with_chat_finalizer(),
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
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.resolved_intent =
        format!("Check whether archive member nested/config.ini is present in {archive}.");
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = archive.to_string();
    route.output_contract.delivery_required = false;
    route.route_reason = "capability_ref=archive.list".to_string();
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    assert_empty_planner_actions_stay_empty(
        &state,
        &route,
        &loop_state,
        "check archive member existence",
        "nested/config.ini in scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip",
    );

    let args = assert_planner_supplied_skill_call_preserved(
        &state,
        &route,
        &loop_state,
        "check archive member existence",
        Some(archive),
        Some(archive),
        "archive_basic",
        "list",
        json!({
            "action": "list",
            "archive": archive,
        }),
    );

    assert_eq!(args.get("archive").and_then(Value::as_str), Some(archive));
}

#[test]
fn archive_entry_existence_scalar_shape_uses_archive_list() {
    let state = test_state_with_enabled_skills(&["archive_basic"]);
    let archive = "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip";
    let mut route = route_result(
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.resolved_intent =
        format!("Check whether archive member notes.txt is present in {archive}.");
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = archive.to_string();
    route.output_contract.delivery_required = false;
    route.route_reason = "capability_ref=archive.list".to_string();
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    assert_empty_planner_actions_stay_empty(
        &state,
        &route,
        &loop_state,
        "check archive member scalar existence",
        "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip 에 notes.txt 가 있는지만 말해. 압축 풀지 마.",
    );

    let args = assert_planner_supplied_skill_call_preserved(
        &state,
        &route,
        &loop_state,
        "check archive member scalar existence",
        Some(archive),
        Some(archive),
        "archive_basic",
        "list",
        json!({
            "action": "list",
            "archive": archive,
        }),
    );

    assert_eq!(args.get("archive").and_then(Value::as_str), Some(archive));
}

#[test]
fn archive_file_existence_without_member_target_still_stats_archive() {
    let archive = "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip";
    let mut route = route_result(
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.resolved_intent = format!("Check whether {archive} exists.");
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = archive.to_string();
    route.output_contract.delivery_required = false;
    route.route_reason = "capability_ref=filesystem.stat_paths".to_string();
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let args = assert_planner_supplied_tool_call_preserved(
        &test_state(),
        &route,
        &loop_state,
        "check archive file existence",
        Some("Check whether scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip exists."),
        Some(archive),
        "fs_basic",
        "stat_paths",
        json!({
            "action": "stat_paths",
            "paths": [archive],
        }),
    );

    assert_eq!(args.get("paths"), Some(&json!([archive])));
}

#[test]
fn existence_with_path_directory_locator_with_file_target_uses_find_path() {
    let root = TempDirGuard::new("existence_dir_locator_file_target");
    fs::create_dir_all(root.path.join("case_only")).expect("mkdir");
    let directory = root.path.join("case_only");
    let directory_path = directory.display().to_string();
    let mut route = route_result(
        crate::AskMode::act_plain(),
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
    route.route_reason = "capability_ref=filesystem.find_entries".to_string();
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let args = assert_planner_supplied_tool_call_preserved(
        &test_state(),
        &route,
        &loop_state,
        "find a file inside a resolved directory",
        Some("Locate report.md within the specified directory and output only its full path."),
        Some(&directory_path),
        "fs_basic",
        "find_entries",
        json!({
            "action": "find_entries",
            "root": directory_path.clone(),
            "pattern": "report.md",
            "target_kind": "file",
        }),
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

#[test]
fn existence_with_path_directory_auto_locator_does_not_parse_history_entries_as_targets() {
    let root = TempDirGuard::new("existence_dir_locator_history_entries");
    fs::create_dir_all(root.path.join("configs")).expect("mkdir configs");
    let directory_path = root.path.join("configs").display().to_string();
    let mut route = route_result(
        crate::AskMode::act_with_chat_finalizer(),
        true,
        OutputResponseShape::Strict,
    );
    route.resolved_intent = "Current task:\n先列出 configs 目录下前 5 个条目名称\n\nMost recent generated output:\nagent_guard.toml\naudio.toml\nbrowser_web_wait_map.json\nchannel_commands.toml\nchannels\n\nNew user instruction:\n看最后一个的基本信息，只回答路径和类型".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = directory_path.clone();
    route.output_contract.delivery_required = false;
    route.route_reason = "capability_ref=filesystem.stat_paths".to_string();
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    assert_empty_planner_actions_stay_empty(
        &test_state(),
        &route,
        &loop_state,
        "follow up on the active ordered list",
        "看最后一个的基本信息，只回答路径和类型",
    );
}

#[test]
fn file_paths_current_workspace_deterministic_plan_uses_name_search() {
    let root = TempDirGuard::new("file_paths_deterministic_plan");
    let script = root.path.join("start-all-bin.sh");
    fs::write(&script, "#!/usr/bin/env bash\n").expect("write script");
    let script_path = script.display().to_string();
    let mut route = route_result(
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::FilePaths;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = "start-all-bin.sh".to_string();
    route.output_contract.delivery_required = false;
    route.route_reason = "capability_ref=filesystem.find_entries".to_string();
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let args = assert_planner_supplied_tool_call_preserved(
        &test_state(),
        &route,
        &loop_state,
        "find a matching file and return its relative path",
        Some("find a matching file and return its relative path"),
        Some(&script_path),
        "fs_basic",
        "find_entries",
        json!({
            "action": "find_entries",
            "pattern": "start-all-bin.sh",
            "target_kind": "file",
        }),
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

#[test]
fn file_paths_path_like_locator_hint_uses_parent_search_scope() {
    let mut route = route_result(
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::FilePaths;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = "case_only/report.md".to_string();
    route.output_contract.delivery_required = false;
    route.route_reason = "capability_ref=filesystem.find_entries".to_string();
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let args = assert_planner_supplied_tool_call_preserved(
        &test_state(),
        &route,
        &loop_state,
        "find path-like locator under its parent scope",
        Some("find path-like locator under its parent scope"),
        None,
        "fs_basic",
        "find_entries",
        json!({
            "action": "find_entries",
            "root": "case_only",
            "pattern": "report.md",
            "target_kind": "file",
        }),
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

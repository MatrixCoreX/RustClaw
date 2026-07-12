use super::*;

fn assert_empty_planner_actions_stay_empty(
    route: &RouteResult,
    loop_state: &LoopState,
    goal: &str,
) {
    let normalized =
        normalize_planned_actions(&test_state(), Some(route), loop_state, goal, None, vec![]);
    assert!(
        normalized.is_empty(),
        "runtime must not inject a git/runtime-status plan before the planner: {normalized:?}"
    );
}

fn assert_runtime_status_planner_action(
    state: &AppState,
    route: &RouteResult,
    loop_state: &LoopState,
    goal: &str,
    kind: &str,
) -> Value {
    assert_planner_supplied_tool_call_preserved(
        state,
        route,
        loop_state,
        goal,
        Some(goal),
        None,
        "system_basic",
        "runtime_status",
        json!({
            "action": "runtime_status",
            "kind": kind,
        }),
    )
}

#[test]
fn chat_wrapped_execution_route_keeps_health_check_observation_only_plan() {
    let loop_state = LoopState::new(2);
    let actions = vec![AgentAction::CallSkill {
        skill: "health_check".to_string(),
        args: serde_json::json!({}),
    }];
    assert!(!should_force_plan_repair(
        Some(&route_result(
            crate::AskMode::act_with_chat_finalizer(),
            false,
            OutputResponseShape::OneSentence,
        )),
        &loop_state,
        &actions,
    ));
}

#[test]
fn non_scalar_route_still_repairs_after_prior_observation_when_delivery_is_empty() {
    let mut loop_state = LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: serde_json::json!({ "command": "ls -l Cargo.toml Cargo.lock" }),
    }];
    assert!(should_force_plan_repair(
        Some(&route_result(
            crate::AskMode::act_with_chat_finalizer(),
            false,
            OutputResponseShape::Free,
        )),
        &loop_state,
        &actions,
    ));
}

#[test]
fn scalar_route_keeps_single_observation_plan_without_followup() {
    let loop_state = LoopState::new(2);
    let actions = vec![AgentAction::CallSkill {
        skill: "git_basic".to_string(),
        args: serde_json::json!({ "action": "current_branch" }),
    }];
    let route = route_result(
        crate::AskMode::act_plain(),
        false,
        OutputResponseShape::Scalar,
    );
    assert!(
        !should_force_plan_repair(Some(&route), &loop_state, &actions),
        "unexpected repair reason: {}",
        repair_reason(Some(&route), &loop_state, Some(&actions))
    );
}

#[test]
fn git_basic_branch_alias_scalar_route_normalizes_to_current_branch() {
    let route = route_result(
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    let actions = vec![AgentAction::CallSkill {
        skill: "git_basic".to_string(),
        args: serde_json::json!({ "action": "branches" }),
    }];

    let normalized = normalize_git_basic_schema_aliases(Some(&route), actions);

    assert!(matches!(
        &normalized[0],
        AgentAction::CallSkill { skill, args }
            if skill == "git_basic"
                && args.get("action").and_then(Value::as_str) == Some("current_branch")
    ));
}

#[test]
fn git_basic_branch_alias_non_scalar_route_normalizes_to_branch() {
    let route = route_result(crate::AskMode::act_plain(), true, OutputResponseShape::Free);
    let actions = vec![AgentAction::CallSkill {
        skill: "git_basic".to_string(),
        args: serde_json::json!({ "action": "branches" }),
    }];

    let normalized = normalize_git_basic_schema_aliases(Some(&route), actions);

    assert!(matches!(
        &normalized[0],
        AgentAction::CallSkill { skill, args }
            if skill == "git_basic"
                && args.get("action").and_then(Value::as_str) == Some("branch")
    ));
}

#[test]
fn git_basic_show_head_path_target_normalizes_to_show_file_at_rev() {
    let route = route_result(crate::AskMode::act_plain(), true, OutputResponseShape::Free);
    let actions = vec![AgentAction::CallSkill {
        skill: "git_basic".to_string(),
        args: serde_json::json!({ "action": "show", "target": "HEAD:README.md" }),
    }];

    let normalized = normalize_git_basic_schema_aliases(Some(&route), actions);

    assert!(matches!(
        &normalized[0],
        AgentAction::CallSkill { skill, args }
            if skill == "git_basic"
                && args.get("action").and_then(Value::as_str) == Some("show_file_at_rev")
                && args.get("target").and_then(Value::as_str) == Some("HEAD")
                && args.get("path").and_then(Value::as_str) == Some("README.md")
    ));
}

#[test]
fn git_show_file_at_rev_capability_rewrites_fs_read_to_git_basic() {
    let mut route = route_result(crate::AskMode::act_plain(), true, OutputResponseShape::Free);
    route.route_reason = "capability_ref=git.show_file_at_rev".to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "fs_basic".to_string(),
        args: serde_json::json!({ "action": "read_text_range", "path": "README.md" }),
    }];

    let normalized = rewrite_git_show_file_at_rev_capability_fs_reads(Some(&route), actions);

    assert!(matches!(
        &normalized[0],
        AgentAction::CallSkill { skill, args }
            if skill == "git_basic"
                && args.get("action").and_then(Value::as_str) == Some("show_file_at_rev")
                && args.get("target").and_then(Value::as_str) == Some("HEAD")
                && args.get("path").and_then(Value::as_str) == Some("README.md")
    ));
}

#[test]
fn git_status_run_cmd_rewrites_to_structured_git_basic() {
    let state = test_state_with_enabled_skills(&["git_basic", "run_cmd"]);
    let workspace_root = state.skill_rt.workspace_root.display().to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: serde_json::json!({
            "command": "git status --porcelain",
            "cwd": workspace_root
        }),
    }];

    let normalized = rewrite_readonly_git_run_cmd_to_git_basic(&state, None, actions);

    assert!(matches!(
        &normalized[0],
        AgentAction::CallSkill { skill, args }
            if skill == "git_basic"
                && args.get("action").and_then(Value::as_str) == Some("status")
    ));
}

#[test]
fn git_status_run_cmd_with_foreign_cwd_is_preserved() {
    let state = test_state_with_enabled_skills(&["git_basic", "run_cmd"]);
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: serde_json::json!({
            "command": "git status --porcelain",
            "cwd": "/tmp/other-repository"
        }),
    }];

    let normalized = rewrite_readonly_git_run_cmd_to_git_basic(&state, None, actions);

    assert!(matches!(
        &normalized[0],
        AgentAction::CallSkill { skill, args }
            if skill == "run_cmd"
                && args.get("cwd").and_then(Value::as_str) == Some("/tmp/other-repository")
    ));
}

#[test]
fn git_status_call_capability_rewrites_after_resolution() {
    let state = test_state_with_enabled_skills(&["git_basic", "run_cmd"]);
    let normalized = normalize_planned_actions(
        &state,
        None,
        &LoopState::new(1),
        "machine field request",
        None,
        vec![AgentAction::CallCapability {
            capability: "system.run_command".to_string(),
            args: serde_json::json!({ "command": "git status --porcelain" }),
        }],
    );

    assert!(matches!(
        &normalized[0],
        AgentAction::CallSkill { skill, args }
            if skill == "git_basic"
                && args.get("action").and_then(Value::as_str) == Some("status")
    ));
}

#[test]
fn literal_git_status_run_cmd_is_preserved() {
    let state = test_state_with_enabled_skills(&["git_basic", "run_cmd"]);
    let mut args = serde_json::json!({ "command": "git status --porcelain" });
    args.as_object_mut().unwrap().insert(
        crate::agent_engine::CLAWD_LITERAL_COMMAND_ARG.to_string(),
        Value::Bool(true),
    );
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args,
    }];

    let normalized = rewrite_readonly_git_run_cmd_to_git_basic(&state, None, actions);

    assert!(matches!(
        &normalized[0],
        AgentAction::CallSkill { skill, args }
            if skill == "run_cmd"
                && args.get("command").and_then(Value::as_str) == Some("git status --porcelain")
    ));
}

#[test]
fn git_repository_state_remote_request_plans_git_remote_action() {
    let loop_state = LoopState::new(2);
    let mut route = route_result(
        crate::AskMode::respond_trace(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.resolved_intent = "capability_ref=git.remote".to_string();
    route.route_reason = "capability_ref=git.remote".to_string();

    let args = assert_planner_supplied_skill_call_preserved(
        &test_state(),
        &route,
        &loop_state,
        "git remote capability",
        Some("ordinary request text"),
        None,
        "git_basic",
        "remote",
        json!({"action": "remote"}),
    );

    assert_eq!(args.get("action").and_then(Value::as_str), Some("remote"));
}

#[test]
fn git_repository_state_contract_without_machine_token_defers_to_planner() {
    let loop_state = LoopState::new(2);
    let mut route = route_result(
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::GitRepositoryState;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;

    assert_empty_planner_actions_stay_empty(&route, &loop_state, "semantic contract only");
}

#[test]
fn git_repository_state_status_capability_ref_plans_git_status_action() {
    let loop_state = LoopState::new(2);
    let mut route = route_result(
        crate::AskMode::respond_trace(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.resolved_intent = "capability_ref=git.status".to_string();
    route.route_reason = "capability_ref=git.status".to_string();

    let args = assert_planner_supplied_skill_call_preserved(
        &test_state(),
        &route,
        &loop_state,
        "git status capability",
        Some("ordinary request text"),
        None,
        "git_basic",
        "status",
        json!({"action": "status"}),
    );

    assert_eq!(args.get("action").and_then(Value::as_str), Some("status"));
}

#[test]
fn git_repository_state_one_sentence_branch_summary_defers_to_planner() {
    let loop_state = LoopState::new(2);
    let mut route = route_result(
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::GitRepositoryState;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;

    assert_empty_planner_actions_stay_empty(&route, &loop_state, "semantic contract only");
}

#[test]
fn git_repository_state_strict_branch_summary_defers_to_planner() {
    let loop_state = LoopState::new(2);
    let mut route = route_result(
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::GitRepositoryState;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;

    assert_empty_planner_actions_stay_empty(&route, &loop_state, "semantic contract only");
}

#[test]
fn recent_scalar_current_workspace_plans_git_branch_without_nl_matching() {
    let state = test_state_with_enabled_skills(&["git_basic", "run_cmd"]);
    let loop_state = LoopState::new(1);
    let mut route = route_result(
        crate::AskMode::respond_trace(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RecentScalarEqualityCheck;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;

    route.route_reason = "capability_ref=git.current_branch".to_string();
    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &loop_state,
        "semantic contract only",
        None,
        vec![
            AgentAction::CallSkill {
                skill: "git_basic".to_string(),
                args: json!({"action": "current_branch"}),
            },
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec!["last_output".to_string()],
            },
            AgentAction::Respond {
                content: "{{last_output}}".to_string(),
            },
        ],
    );

    assert_eq!(normalized.len(), 3);
    let AgentAction::CallSkill { skill, args } = &normalized[0] else {
        panic!("expected git_basic action, got {:?}", normalized[0]);
    };
    assert_eq!(skill, "git_basic");
    assert_eq!(
        args.get("action").and_then(Value::as_str),
        Some("current_branch")
    );
}

#[test]
fn recent_scalar_current_workspace_git_observation_satisfies_repair_guard() {
    let state = test_state_with_enabled_skills(&["git_basic", "run_cmd"]);
    let loop_state = LoopState::new(1);
    let mut route = route_result(
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RecentScalarEqualityCheck;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "git_basic".to_string(),
            args: serde_json::json!({ "action": "current_branch" }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    assert!(!should_force_actionable_plan_repair(
        &state,
        Some(&route),
        &loop_state,
        &actions
    ));
}

#[test]
fn raw_command_output_route_keeps_single_run_cmd_plan_without_followup() {
    let loop_state = LoopState::new(2);
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: serde_json::json!({ "command": "ls", "cwd": "/tmp/rustclaw-workspace" }),
    }];
    let mut route = route_result(
        crate::AskMode::act_plain(),
        false,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    assert!(!should_force_plan_repair(
        Some(&route),
        &loop_state,
        &actions,
    ));
}

#[test]
fn runtime_status_scalar_patch_plans_current_user_system_basic_status() {
    let state = test_state_with_enabled_skills(&["system_basic"]);
    let loop_state = LoopState::new(1);
    let mut route = route_result(
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.route_reason = "capability_ref=system.runtime_status".to_string();

    let args = assert_runtime_status_planner_action(
        &state,
        &route,
        &loop_state,
        "return current user",
        "current_user",
    );

    assert_eq!(
        args.get("action").and_then(Value::as_str),
        Some("runtime_status")
    );
    assert_eq!(
        args.get("kind").and_then(Value::as_str),
        Some("current_user")
    );
}

#[test]
fn runtime_status_scalar_string_patch_plans_current_user_system_basic_status() {
    let state = test_state_with_enabled_skills(&["system_basic"]);
    let loop_state = LoopState::new(1);
    let mut route = route_result(
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.route_reason = "capability_ref=system.runtime_status".to_string();

    let args = assert_runtime_status_planner_action(
        &state,
        &route,
        &loop_state,
        "return current user",
        "current_user",
    );

    assert_eq!(
        args.get("action").and_then(Value::as_str),
        Some("runtime_status")
    );
    assert_eq!(
        args.get("kind").and_then(Value::as_str),
        Some("current_user")
    );
}

#[test]
fn runtime_status_scalar_patch_does_not_depend_on_route_trace() {
    let state = test_state_with_enabled_skills(&["system_basic"]);
    let loop_state = LoopState::new(1);
    let mut route = route_result(
        crate::AskMode::respond_trace(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.route_reason = "capability_ref=system.runtime_status".to_string();

    let args = assert_runtime_status_planner_action(
        &state,
        &route,
        &loop_state,
        "return current user",
        "current_user",
    );

    assert_eq!(
        args.get("action").and_then(Value::as_str),
        Some("runtime_status")
    );
}

#[test]
fn runtime_status_scalar_patch_prefers_system_basic_when_available() {
    let state = test_state_with_enabled_skills(&["run_cmd", "system_basic"]);
    let loop_state = LoopState::new(1);
    let mut route = route_result(
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.route_reason = "capability_ref=system.runtime_status".to_string();

    let args = assert_runtime_status_planner_action(
        &state,
        &route,
        &loop_state,
        "return current user",
        "current_user",
    );

    assert_eq!(
        args.get("action").and_then(Value::as_str),
        Some("runtime_status")
    );
    assert_eq!(
        args.get("kind").and_then(Value::as_str),
        Some("current_user")
    );
}

#[test]
fn runtime_status_scalar_patch_plans_hostname_system_basic_status() {
    let state = test_state_with_enabled_skills(&["run_cmd", "system_basic"]);
    let loop_state = LoopState::new(1);
    let mut route = route_result(
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.route_reason = "capability_ref=system.runtime_status".to_string();

    let args = assert_runtime_status_planner_action(
        &state,
        &route,
        &loop_state,
        "return current hostname",
        "host_name",
    );

    assert_eq!(
        args.get("action").and_then(Value::as_str),
        Some("runtime_status")
    );
    assert_eq!(args.get("kind").and_then(Value::as_str), Some("host_name"));
}

#[tokio::test]
async fn runtime_status_query_reaches_planner_without_literal_command_fast_path() {
    let mut state = test_state_with_enabled_skills(&["run_cmd", "system_basic"]);
    state.policy.command_intent.standalone_commands = vec!["hostname".to_string()];
    let prompt = "只输出当前机器 hostname，不要解释";
    let task = ClaimedTask {
        task_id: "runtime-hostname-fast-path".to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: json!({ "text": prompt }).to_string(),
    };
    let mut route = route_result(
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.resolved_intent = "return current hostname".to_string();
    let analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::StatusQuery),
        target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
        should_interrupt_active_run: false,
        state_patch: Some(json!({
            "runtime_status_query": {"kind": "host_name", "scope": "system"}
        })),
        attachment_processing_required: false,
    };
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;
    let policy = super::super::super::support::load_agent_loop_guard_policy(&state);

    let err = super::super::plan_round_actions(
        &state,
        &task,
        &route.resolved_intent,
        prompt,
        &policy,
        &loop_state,
        Some(&analysis),
        None,
        Some(&route),
        None,
    )
    .await
    .expect_err("runtime status query should reach planner instead of pre-LLM capability choice");

    assert!(
        err.contains("required prompt missing"),
        "expected missing planner prompt after deterministic runtime-status removal, got: {err}"
    );
    assert!(
        !err.contains("plan_deterministic_runtime_status_scalar"),
        "old runtime-status deterministic fallback leaked into planner error: {err}"
    );
    assert!(
        !err.contains("plan_deterministic_explicit_command_run_cmd"),
        "runtime status query fell back to literal command fast path: {err}"
    );
}

#[test]
fn runtime_status_scalar_patch_maps_kernel_release_to_uname_r() {
    let state = test_state_with_enabled_skills(&["run_cmd", "system_basic"]);
    let loop_state = LoopState::new(1);
    let mut route = route_result(
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.route_reason = "capability_ref=system.runtime_status".to_string();

    let args = assert_runtime_status_planner_action(
        &state,
        &route,
        &loop_state,
        "return kernel release",
        "kernel_release",
    );

    assert_eq!(
        args.get("action").and_then(Value::as_str),
        Some("runtime_status")
    );
    assert_eq!(
        args.get("kind").and_then(Value::as_str),
        Some("kernel_release")
    );
}

#[test]
fn raw_command_output_runtime_status_plan_keeps_system_basic_when_available() {
    let state = test_state_with_enabled_skills(&["run_cmd", "system_basic"]);
    let loop_state = LoopState::new(1);
    let mut route = route_result(
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route.output_contract.locator_kind = OutputLocatorKind::None;

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &loop_state,
        "runtime scalar",
        None,
        vec![AgentAction::CallTool {
            tool: "system_basic".to_string(),
            args: json!({
                "action": "runtime_status",
                "kind": "current_user"
            }),
        }],
    );

    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallTool { tool, args } => {
            assert_eq!(tool, "system_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("runtime_status")
            );
            assert_eq!(
                args.get("kind").and_then(Value::as_str),
                Some("current_user")
            );
        }
        other => panic!("expected system_basic runtime_status, got {other:?}"),
    }
}

#[test]
fn raw_command_output_runtime_status_planner_tool_choice_is_not_fallback_rewritten() {
    let state = test_state_with_enabled_skills(&["run_cmd"]);
    let loop_state = LoopState::new(1);
    let mut route = route_result(
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route.output_contract.locator_kind = OutputLocatorKind::None;

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &loop_state,
        "runtime scalar",
        None,
        vec![AgentAction::CallTool {
            tool: "system_basic".to_string(),
            args: json!({
                "action": "runtime_status",
                "kind": "current_user"
            }),
        }],
    );

    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallTool { tool, args } => {
            assert_eq!(tool, "system_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("runtime_status")
            );
            assert_eq!(
                args.get("kind").and_then(Value::as_str),
                Some("current_user")
            );
        }
        other => panic!("expected planner-supplied system_basic action, got {other:?}"),
    }
}

#[test]
fn file_delivery_route_allows_plain_not_found_terminal_reply() {
    let loop_state = LoopState::new(2);
    let actions = vec![AgentAction::Respond {
        content: "未找到该文件。".to_string(),
    }];
    assert!(!should_force_plan_repair(
        Some(&delivery_route_result()),
        &loop_state,
        &actions,
    ));
}

#[test]
fn ops_recipe_apply_phase_without_mutation_forces_plan_repair() {
    let mut loop_state = LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        ..Default::default()
    };
    let actions = vec![AgentAction::CallSkill {
        skill: "http_basic".to_string(),
        args: serde_json::json!({ "action": "get", "url": "http://127.0.0.1:60703/" }),
    }];
    assert!(should_force_plan_repair(
        Some(&route_result(
            crate::AskMode::act_plain(),
            false,
            OutputResponseShape::Free,
        )),
        &loop_state,
        &actions,
    ));
}

#[test]
fn ops_recipe_apply_phase_without_mutation_uses_specific_repair_reason() {
    let mut loop_state = LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        ..Default::default()
    };
    let actions = vec![AgentAction::CallSkill {
        skill: "http_basic".to_string(),
        args: serde_json::json!({ "action": "get", "url": "http://127.0.0.1:60703/" }),
    }];
    assert_eq!(
        repair_reason(
            Some(&route_result(
                crate::AskMode::act_plain(),
                false,
                OutputResponseShape::Free,
            )),
            &loop_state,
            Some(&actions),
        ),
        "ops_closed_loop_apply_requires_mutation"
    );
}

#[test]
fn ops_recipe_apply_phase_with_mutation_keeps_plan() {
    let mut loop_state = LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        ..Default::default()
    };
    let actions = vec![
        AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: serde_json::json!({ "path": "document/index.html" }),
        },
        AgentAction::CallSkill {
            skill: "write_file".to_string(),
            args: serde_json::json!({ "path": "document/index.html", "content": "ops-repair-ok\n" }),
        },
    ];
    assert!(!should_force_plan_repair(
        Some(&route_result(
            crate::AskMode::act_plain(),
            false,
            OutputResponseShape::Scalar,
        )),
        &loop_state,
        &actions,
    ));
}

#[test]
fn config_change_profile_without_post_change_validation_forces_repair() {
    let mut loop_state = LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        profile: crate::execution_recipe::ExecutionRecipeProfile::ConfigChange,
        phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        ..Default::default()
    };
    let actions = vec![
        AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: serde_json::json!({ "path": "configs/config.toml" }),
        },
        AgentAction::CallSkill {
            skill: "write_file".to_string(),
            args: serde_json::json!({ "path": "configs/config.toml", "content": "[tools]\nallow_sudo=false\n" }),
        },
    ];
    assert!(should_force_plan_repair(
        Some(&route_result(
            crate::AskMode::act_plain(),
            false,
            OutputResponseShape::Free,
        )),
        &loop_state,
        &actions,
    ));
    assert_eq!(
        repair_reason(
            Some(&route_result(
                crate::AskMode::act_plain(),
                false,
                OutputResponseShape::Free,
            )),
            &loop_state,
            Some(&actions),
        ),
        "config_change_requires_post_change_validation"
    );
}

#[test]
fn skill_authoring_profile_requires_integration_validation_not_readback_only() {
    let mut loop_state = LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        profile: crate::execution_recipe::ExecutionRecipeProfile::SkillAuthoring,
        phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        ..Default::default()
    };
    let actions = vec![
        AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: serde_json::json!({ "path": "crates/skills/foo/INTERFACE.md" }),
        },
        AgentAction::CallSkill {
            skill: "write_file".to_string(),
            args: serde_json::json!({ "path": "crates/skills/foo/INTERFACE.md", "content": "# Foo\n" }),
        },
        AgentAction::CallSkill {
            skill: "http_basic".to_string(),
            args: serde_json::json!({ "action": "get", "url": "http://127.0.0.1:62078/" }),
        },
    ];
    assert!(should_force_plan_repair(
        Some(&route_result(
            crate::AskMode::act_plain(),
            false,
            OutputResponseShape::Free,
        )),
        &loop_state,
        &actions,
    ));
    assert_eq!(
        repair_reason(
            Some(&route_result(
                crate::AskMode::act_plain(),
                false,
                OutputResponseShape::Free,
            )),
            &loop_state,
            Some(&actions),
        ),
        "skill_authoring_requires_integration_validation"
    );
}

#[test]
fn code_change_profile_requires_verification_not_readback_only() {
    let mut loop_state = LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
        phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        ..Default::default()
    };
    let actions = vec![
        AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: serde_json::json!({ "path": "crates/clawd/src/main.rs" }),
        },
        AgentAction::CallSkill {
            skill: "write_file".to_string(),
            args: serde_json::json!({ "path": "crates/clawd/src/main.rs", "content": "fn main() {}\n" }),
        },
        AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: serde_json::json!({ "path": "crates/clawd/src/main.rs" }),
        },
    ];
    assert!(should_force_plan_repair(
        Some(&route_result(
            crate::AskMode::act_plain(),
            false,
            OutputResponseShape::Free,
        )),
        &loop_state,
        &actions,
    ));
    assert_eq!(
        repair_reason(
            Some(&route_result(
                crate::AskMode::act_plain(),
                false,
                OutputResponseShape::Free,
            )),
            &loop_state,
            Some(&actions),
        ),
        "code_change_requires_verification"
    );
}

#[test]
fn code_change_profile_done_allows_terminal_response_without_extra_validation_step() {
    let mut loop_state = LoopState::new(4);
    loop_state.has_tool_or_skill_output = true;
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
        phase: crate::execution_recipe::ExecutionRecipePhase::Done,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        saw_mutation: true,
        saw_validation: true,
        ..Default::default()
    };
    loop_state.latest_validation_result = Some(serde_json::json!({
        "schema_version": 1,
        "source": "agent_loop_step_validation",
        "status": "passed",
        "status_code": "validation_passed",
        "skill": "http_basic",
        "global_step": 8,
        "step_in_round": 2
    }));
    let actions = vec![AgentAction::Respond {
        content: "VALIDATION_PASSED".to_string(),
    }];

    assert!(!should_force_plan_repair(
        Some(&route_result(
            crate::AskMode::act_plain(),
            false,
            OutputResponseShape::Free,
        )),
        &loop_state,
        &actions,
    ));
}

#[test]
fn package_change_profile_without_post_install_validation_forces_repair() {
    let mut loop_state = LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        profile: crate::execution_recipe::ExecutionRecipeProfile::PackageChange,
        target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::System,
        phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        ..Default::default()
    };
    let actions = vec![
        AgentAction::CallSkill {
            skill: "package_manager".to_string(),
            args: serde_json::json!({ "action": "detect" }),
        },
        AgentAction::CallSkill {
            skill: "package_manager".to_string(),
            args: serde_json::json!({ "action": "install", "package": "jq", "dry_run": false }),
        },
    ];
    assert!(should_force_plan_repair(
        Some(&route_result(
            crate::AskMode::act_plain(),
            false,
            OutputResponseShape::Free,
        )),
        &loop_state,
        &actions,
    ));
    assert_eq!(
        repair_reason(
            Some(&route_result(
                crate::AskMode::act_plain(),
                false,
                OutputResponseShape::Free,
            )),
            &loop_state,
            Some(&actions),
        ),
        "package_change_requires_validation"
    );
}

#[test]
fn database_change_profile_keeps_schema_validation_after_execute() {
    let mut loop_state = LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        profile: crate::execution_recipe::ExecutionRecipeProfile::DatabaseChange,
        target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::CurrentRepo,
        phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        ..Default::default()
    };
    let actions = vec![
        AgentAction::CallSkill {
            skill: "db_basic".to_string(),
            args: serde_json::json!({
                "action": "sqlite_execute",
                "db_path": "data/app.db",
                "sql": "UPDATE users SET active=1",
                "confirm": true
            }),
        },
        AgentAction::CallSkill {
            skill: "db_basic".to_string(),
            args: serde_json::json!({
                "action": "schema_version",
                "db_path": "data/app.db"
            }),
        },
    ];
    assert!(
        !should_force_plan_repair(
            Some(&route_result(
                crate::AskMode::act_plain(),
                false,
                OutputResponseShape::Free,
            )),
            &loop_state,
            &actions,
        ),
        "unexpected repair reason: {}",
        repair_reason(
            Some(&route_result(
                crate::AskMode::act_plain(),
                false,
                OutputResponseShape::Free,
            )),
            &loop_state,
            Some(&actions),
        )
    );
}

#[test]
fn code_change_profile_with_structured_cargo_check_keeps_plan() {
    let mut loop_state = LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
        phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        ..Default::default()
    };
    let actions = vec![
        AgentAction::CallSkill {
            skill: "write_file".to_string(),
            args: serde_json::json!({ "path": "crates/clawd/src/main.rs", "content": "fn main() {}\n" }),
        },
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: serde_json::json!({
                "command": "cargo check -p clawd",
                "_clawd_validation": {
                    "profile": "code_change",
                    "validator_type": "build",
                    "validated_target": "clawd"
                }
            }),
        },
    ];
    assert!(!should_force_plan_repair(
        Some(&route_result(
            crate::AskMode::act_plain(),
            false,
            OutputResponseShape::Scalar,
        )),
        &loop_state,
        &actions,
    ));
}

#[test]
fn code_change_profile_with_run_cmd_cargo_check_keeps_plan() {
    let mut loop_state = LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
        phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        ..Default::default()
    };
    let actions = vec![
        AgentAction::CallSkill {
            skill: "write_file".to_string(),
            args: serde_json::json!({ "path": "crates/clawd/src/main.rs", "content": "fn main() {}\n" }),
        },
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: serde_json::json!({ "command": "cargo check -p clawd" }),
        },
    ];
    assert!(!should_force_plan_repair(
        Some(&route_result(
            crate::AskMode::act_plain(),
            false,
            OutputResponseShape::Scalar,
        )),
        &loop_state,
        &actions,
    ));
}

#[test]
fn current_repo_scope_rejects_external_absolute_path() {
    let mut loop_state = LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
        target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::CurrentRepo,
        phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        ..Default::default()
    };
    let actions = vec![
        AgentAction::CallSkill {
            skill: "write_file".to_string(),
            args: serde_json::json!({ "path": "/opt/other-project/main.rs", "content": "fn main() {}\n" }),
        },
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: serde_json::json!({
                "command": "cargo check -p clawd",
                "_clawd_validation": {
                    "profile": "code_change",
                    "validator_type": "build",
                    "validated_target": "tools/demo"
                }
            }),
        },
    ];
    assert!(should_force_plan_repair(
        Some(&route_result(
            crate::AskMode::act_plain(),
            false,
            OutputResponseShape::Free,
        )),
        &loop_state,
        &actions,
    ));
    assert_eq!(
        repair_reason(
            Some(&route_result(
                crate::AskMode::act_plain(),
                false,
                OutputResponseShape::Free,
            )),
            &loop_state,
            Some(&actions),
        ),
        "current_repo_scope_rejects_external_target"
    );
}

#[test]
fn external_workspace_scope_requires_explicit_external_target() {
    let mut loop_state = LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
        target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::ExternalWorkspace,
        phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        ..Default::default()
    };
    let actions = vec![
        AgentAction::CallSkill {
            skill: "write_file".to_string(),
            args: serde_json::json!({ "path": "crates/clawd/src/main.rs", "content": "fn main() {}\n" }),
        },
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: serde_json::json!({
                "command": "cargo check -p clawd",
                "_clawd_validation": {
                    "profile": "code_change",
                    "validator_type": "build",
                    "validated_target": "tools/demo"
                }
            }),
        },
    ];
    assert!(should_force_plan_repair(
        Some(&route_result(
            crate::AskMode::act_plain(),
            false,
            OutputResponseShape::Free,
        )),
        &loop_state,
        &actions,
    ));
    assert_eq!(
        repair_reason(
            Some(&route_result(
                crate::AskMode::act_plain(),
                false,
                OutputResponseShape::Free,
            )),
            &loop_state,
            Some(&actions),
        ),
        "external_workspace_requires_explicit_target"
    );
}

#[test]
fn greenfield_scope_requires_creation_step_before_validation() {
    let mut loop_state = LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
        target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::Greenfield,
        phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        ..Default::default()
    };
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: serde_json::json!({ "command": "cargo check -p clawd" }),
    }];
    assert!(should_force_plan_repair(
        Some(&route_result(
            crate::AskMode::act_plain(),
            false,
            OutputResponseShape::Free,
        )),
        &loop_state,
        &actions,
    ));
    assert_eq!(
        repair_reason(
            Some(&route_result(
                crate::AskMode::act_plain(),
                false,
                OutputResponseShape::Free,
            )),
            &loop_state,
            Some(&actions),
        ),
        "greenfield_requires_artifact_creation"
    );
}

#[test]
fn greenfield_scope_with_make_dir_and_write_file_keeps_plan() {
    let mut loop_state = LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
        target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::Greenfield,
        phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        ..Default::default()
    };
    let actions = vec![
        AgentAction::CallSkill {
            skill: "make_dir".to_string(),
            args: serde_json::json!({ "path": "tools/demo" }),
        },
        AgentAction::CallSkill {
            skill: "write_file".to_string(),
            args: serde_json::json!({ "path": "tools/demo/main.rs", "content": "fn main() {}\n" }),
        },
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: serde_json::json!({
                "command": "cargo check -p clawd",
                "_clawd_validation": {
                    "profile": "code_change",
                    "validator_type": "build",
                    "validated_target": "tools/demo"
                }
            }),
        },
    ];
    let route = route_result(
        crate::AskMode::act_plain(),
        false,
        OutputResponseShape::Scalar,
    );
    assert!(
        !should_force_plan_repair(Some(&route), &loop_state, &actions),
        "unexpected repair reason: {}",
        repair_reason(Some(&route), &loop_state, Some(&actions))
    );
}

#[test]
fn greenfield_scope_with_fs_basic_make_dir_and_write_text_keeps_plan() {
    let mut loop_state = LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
        target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::Greenfield,
        phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        ..Default::default()
    };
    let project_dir = "/home/guagua/rustclaw/run/nl_eval_tmp/codex_cli_continuous_20260711_new";
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": "make_dir",
                "path": project_dir
            }),
        },
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": "write_text",
                "path": format!("{project_dir}/calc_core.py"),
                "content": "def add(a, b):\n    return a + b\n"
            }),
        },
        AgentAction::CallTool {
            tool: "run_cmd".to_string(),
            args: serde_json::json!({
                "command": "python3 test_calc_core.py",
                "cwd": project_dir,
                "_clawd_validation": {
                    "profile": "code_change",
                    "validator_type": "test",
                    "validated_target": project_dir
                }
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];
    let mut route = route_result(
        crate::AskMode::act_plain(),
        false,
        OutputResponseShape::Strict,
    );
    route.route_reason =
        "execution_recipe_target_locator_preserved_for_agent_loop; executable_contract_preserved_for_agent_loop"
            .to_string();
    assert!(
        !should_force_plan_repair(Some(&route), &loop_state, &actions),
        "unexpected repair reason: {}",
        repair_reason(Some(&route), &loop_state, Some(&actions))
    );
}

#[test]
fn external_workspace_scope_persists_across_rounds_without_repeating_path() {
    let mut loop_state = LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
        target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::ExternalWorkspace,
        phase: crate::execution_recipe::ExecutionRecipePhase::Validate,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        saw_mutation: true,
        saw_external_target: true,
        ..Default::default()
    };
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: serde_json::json!({
            "command": "cargo check",
            "_clawd_validation": {
                "profile": "code_change",
                "validator_type": "build",
                "validated_target": "external_workspace"
            }
        }),
    }];
    assert!(!should_force_plan_repair(
        Some(&route_result(
            crate::AskMode::act_plain(),
            false,
            OutputResponseShape::Scalar,
        )),
        &loop_state,
        &actions,
    ));
}

#[test]
fn greenfield_scope_persists_creation_across_rounds() {
    let mut loop_state = LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
        target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::Greenfield,
        phase: crate::execution_recipe::ExecutionRecipePhase::Validate,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        saw_mutation: true,
        saw_greenfield_creation: true,
        ..Default::default()
    };
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: serde_json::json!({
            "command": "cargo check -p clawd",
            "_clawd_validation": {
                "profile": "code_change",
                "validator_type": "build",
                "validated_target": "greenfield_project"
            }
        }),
    }];
    assert!(!should_force_plan_repair(
        Some(&route_result(
            crate::AskMode::act_plain(),
            false,
            OutputResponseShape::Scalar,
        )),
        &loop_state,
        &actions,
    ));
}

#[test]
fn content_evidence_route_allows_respond_only_after_prior_observation() {
    let mut loop_state = LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    let actions = vec![AgentAction::Respond {
        content: "grounded final answer".to_string(),
    }];
    assert!(!should_force_plan_repair(
        Some(&route_result(
            crate::AskMode::act_with_chat_finalizer(),
            true,
            OutputResponseShape::Free,
        )),
        &loop_state,
        &actions,
    ));
}

#[test]
fn extracts_xml_call_skill_markup_into_step_values() {
    let raw = r#"<tool_call>
<invoke name="call_skill">
<parameter name="skill">list_dir</parameter>
<parameter name="args">{"path": "/tmp"}</parameter>
</invoke>
</tool_call>"#;
    assert_eq!(
        super::super::extract_xml_tool_call_steps(raw),
        vec![json!({
            "type": "call_skill",
            "skill": "list_dir",
            "args": { "path": "/tmp" }
        })]
    );
}

#[test]
fn extracts_xml_direct_skill_invoke_markup_into_step_values() {
    let raw = r#"<tool_call>
<invoke name="fs_search">
<parameter name="args">{"action":"find_name","pattern":"README"}</parameter>
</invoke>
</tool_call>"#;
    assert_eq!(
        super::super::extract_xml_tool_call_steps(raw),
        vec![json!({
            "type": "call_skill",
            "skill": "fs_search",
            "args": { "action": "find_name", "pattern": "README" }
        })]
    );
}

// ---------- inject_synthesize_answer_for_bare_placeholder_respond ----------
// 见函数 doc：runtime 兜底，把兼容模型偶发吐出的裸 placeholder respond 注入
// 一个 synthesize_answer 节点，关掉裸 placeholder 导致的死循环。

#[test]
fn strips_intermediate_synthesize_before_later_execution() {
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({"action": "path_batch_facts", "paths": ["missing.txt"]}),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({"action": "inventory_dir", "path": "scripts"}),
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let stripped = strip_intermediate_synthesize_before_later_execution(actions);

    assert_eq!(stripped.len(), 3);
    assert!(matches!(
        &stripped[0],
        AgentAction::CallSkill { skill, .. } if skill == "system_basic"
    ));
    assert!(matches!(
        &stripped[1],
        AgentAction::CallSkill { skill, .. } if skill == "system_basic"
    ));
    assert!(matches!(
        &stripped[2],
        AgentAction::Respond { content } if content == "{{last_output}}"
    ));
}

#[test]
fn strips_terminal_placeholder_respond_for_exact_listing_contract() {
    let mut route = route_result(
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::FileNames;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({"action": "inventory_dir", "path": "scripts"}),
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let stripped =
        strip_terminal_placeholder_respond_for_exact_listing_contract(Some(&route), actions);

    assert_eq!(stripped.len(), 1);
    assert!(matches!(
        &stripped[0],
        AgentAction::CallSkill { skill, .. } if skill == "system_basic"
    ));
}

#[test]
fn detects_bare_last_output_placeholder_variants() {
    assert!(is_bare_last_output_placeholder("{{last_output}}"));
    assert!(is_bare_last_output_placeholder("  {{ last_output }}  "));
    assert!(is_bare_last_output_placeholder("{{last_output.hostname}}"));
    assert!(is_bare_last_output_placeholder("{{last_output.foo.bar}}"));
    assert!(is_bare_last_output_placeholder("{{LAST_OUTPUT}}"));
    assert!(is_bare_last_output_placeholder("{{last_output[\"x\"]}}"));
}

#[test]
fn rejects_non_bare_placeholder_content() {
    assert!(!is_bare_last_output_placeholder(
        "hostname is {{last_output}}"
    ));
    assert!(!is_bare_last_output_placeholder("当前用户是 root"));
    assert!(!is_bare_last_output_placeholder(""));
    assert!(!is_bare_last_output_placeholder("{{other}}"));
    assert!(!is_bare_last_output_placeholder("{{lastoutput}}"));
    // last_output 后接非 . / [ 的字符不算同一占位
    assert!(!is_bare_last_output_placeholder("{{last_output_extra}}"));
}

#[test]
fn injects_synthesize_answer_when_respond_is_bare_placeholder() {
    let actions = vec![
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({ "command": "whoami" }),
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];
    let out = inject_synthesize_answer_for_bare_placeholder_respond(
        actions,
        "只输出当前用户名，不要解释",
    );
    assert_eq!(out.len(), 3, "should insert exactly one synth step");
    assert!(matches!(
        &out[0],
        AgentAction::CallSkill { skill, .. } if skill == "run_cmd"
    ));
    match &out[1] {
        AgentAction::SynthesizeAnswer { evidence_refs } => {
            assert_eq!(
                evidence_refs,
                &vec!["last_output".to_string()],
                "synthesize step should point at last_output by default"
            );
        }
        _ => panic!("expected synthesize_answer at index 1, got {:?}", out[1]),
    }
    assert!(matches!(
        &out[2],
        AgentAction::Respond { content } if content == "{{last_output}}"
    ));
}

#[test]
fn appends_terminal_synthesize_for_command_summary_observation_plan() {
    let mut route = route_result(
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::CommandOutputSummary;
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({ "command": "ls scripts" }),
    }];

    let out = super::super::append_terminal_synthesize_for_observation_summary_contract(
        Some(&route),
        actions,
    );

    assert_eq!(out.len(), 2);
    assert!(matches!(
        &out[0],
        AgentAction::CallSkill { skill, .. } if skill == "run_cmd"
    ));
    assert!(matches!(
        &out[1],
        AgentAction::SynthesizeAnswer { evidence_refs }
            if evidence_refs.len() == 1 && evidence_refs[0] == "last_output"
    ));
}

#[test]
fn does_not_append_terminal_synthesize_for_strict_raw_command_output() {
    let mut route = route_result(
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({ "command": "ls scripts" }),
    }];

    let out = super::super::append_terminal_synthesize_for_observation_summary_contract(
        Some(&route),
        actions,
    );

    assert_eq!(out.len(), 1);
    assert!(matches!(
        &out[0],
        AgentAction::CallSkill { skill, .. } if skill == "run_cmd"
    ));
}

#[test]
fn injection_is_idempotent_when_synthesize_already_precedes_respond() {
    let actions = vec![
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({ "command": "whoami" }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];
    let before = actions_as_json(&actions);
    let out = inject_synthesize_answer_for_bare_placeholder_respond(actions, "x");
    assert_eq!(
        actions_as_json(&out),
        before,
        "should not re-inject when synthesize_answer already precedes respond"
    );
}

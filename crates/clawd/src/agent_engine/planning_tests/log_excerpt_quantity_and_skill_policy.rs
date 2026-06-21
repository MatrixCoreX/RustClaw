use super::*;

#[test]
fn contract_rejected_log_analyze_rewrites_to_preferred_excerpt_read() {
    let temp = TempDirGuard::new("contract_preferred_excerpt_read");
    let log_path = temp.path.join("act_plan.log");
    fs::write(&log_path, "{\"level\":\"info\",\"event\":\"ok\"}\n").expect("write fixture log");
    let log_path = log_path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ExcerptKindJudgment;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = log_path.clone();
    let actions = vec![AgentAction::CallSkill {
        skill: "log_analyze".to_string(),
        args: json!({
            "path": log_path,
        }),
    }];

    let normalized = normalize_planned_actions_with_original_and_context(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "judge the current excerpt",
        None,
        None,
        Some(route.output_contract.locator_hint.as_str()),
        actions,
    );

    assert!(!normalized.iter().any(|action| {
        matches!(action, AgentAction::CallSkill { skill, .. } if skill == "log_analyze")
    }));
    let args = normalized
        .iter()
        .find_map(|action| {
            planned_call(action).and_then(|(tool, args)| {
                (tool == "fs_basic"
                    && args.get("action").and_then(Value::as_str) == Some("read_text_range"))
                .then_some(args)
            })
        })
        .expect("expected preferred fs_basic.read_text_range action");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(route.output_contract.locator_hint.as_str())
    );
    assert_eq!(args.get("mode").and_then(Value::as_str), Some("head"));
    assert_eq!(args.get("n").and_then(Value::as_u64), Some(80));
}

#[test]
fn excerpt_contract_keeps_preferred_log_read_when_synthesizing() {
    let temp = TempDirGuard::new("excerpt_contract_keeps_log_read");
    let log_path = temp.path.join("act_plan.log");
    fs::write(&log_path, "{\"level\":\"info\",\"event\":\"ok\"}\n").expect("write fixture log");
    let log_path = log_path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ExcerptKindJudgment;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = log_path.clone();
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": log_path,
                "mode": "tail",
                "n": 3,
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = normalize_planned_actions_with_original_and_context(
        &test_state(),
        Some(&route),
        &LoopState::new(1),
        "judge the current excerpt",
        None,
        None,
        Some(route.output_contract.locator_hint.as_str()),
        actions,
    );

    assert!(!normalized.iter().any(|action| {
        matches!(action, AgentAction::CallSkill { skill, .. } if skill == "log_analyze")
    }));
    let args = expect_planned_call(&normalized[0], "fs_basic", "read_text_range");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(route.output_contract.locator_hint.as_str())
    );
    assert_eq!(args.get("mode").and_then(Value::as_str), Some("tail"));
    assert_eq!(args.get("n").and_then(Value::as_u64), Some(3));
}

#[test]
fn raw_command_output_preserves_fs_basic_grep_text_plan() {
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/logs/app.log".to_string();
    route.output_contract.requires_content_evidence = true;
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "grep_text",
            "path": route.output_contract.locator_hint,
            "query": "ERROR"
        }),
    }];

    let normalized = normalize_planned_actions_with_original_and_context(
        &test_state(),
        Some(&route),
        &LoopState::new(1),
        "bounded content search",
        None,
        None,
        Some(route.output_contract.locator_hint.as_str()),
        actions,
    );

    assert_eq!(normalized.len(), 1, "normalized actions: {normalized:?}");
    let args = expect_planned_call(&normalized[0], "fs_basic", "grep_text");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(route.output_contract.locator_hint.as_str())
    );
    assert_eq!(args.get("query").and_then(Value::as_str), Some("ERROR"));
}

#[test]
fn structured_tool_output_placeholder_is_synthesized_before_respond() {
    let loop_state = LoopState::new(1);
    let route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({"action": "count_entries", "path": "scripts"}),
        },
        AgentAction::Respond {
            content: "scripts has {{last_output}} entries".to_string(),
        },
    ];

    let normalized = normalize_planned_actions(
        &test_state(),
        Some(&route),
        &loop_state,
        "count scripts entries",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 3);
    assert!(matches!(
        &normalized[1],
        AgentAction::SynthesizeAnswer { evidence_refs }
            if evidence_refs == &vec!["last_output".to_string()]
    ));
    assert!(matches!(
        &normalized[2],
        AgentAction::Respond { content } if content == "{{last_output}}"
    ));
}

#[test]
fn fs_basic_append_text_aliases_text_to_content_before_verify() {
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "append_text",
            "path": "document/nl_tool200/group_02/memo.txt",
            "text": "beta"
        }),
    }];

    let normalized = normalize_fs_basic_schema_aliases(actions);
    let args = expect_planned_call(&normalized[0], "fs_basic", "append_text");
    assert_eq!(args.get("content").and_then(Value::as_str), Some("beta"));
    assert!(args.get("text").is_none());
}

#[test]
fn structured_scalar_compare_accepts_fs_basic_count_entries_pair() {
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RecentScalarEqualityCheck;
    let loop_state = LoopState::new(1);
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({"action": "count_entries", "path": "document"}),
        },
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({"action": "count_entries", "path": "scripts"}),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["step_1".to_string(), "step_2".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    assert!(!should_force_actionable_plan_repair(
        &test_state(),
        Some(&route),
        &loop_state,
        &actions
    ));
}

#[test]
fn quantity_comparison_route_accepts_single_count_entries_scalar_plan() {
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    let loop_state = LoopState::new(1);
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({"action": "count_entries", "path": "scripts/nl_tests/fixtures/device_local/docs"}),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    assert!(!should_force_actionable_plan_repair(
        &test_state(),
        Some(&route),
        &loop_state,
        &actions
    ));
}

#[test]
fn command_output_summary_keeps_planned_fs_count_entries_actions() {
    let state = test_state_with_enabled_skills(&["fs_basic", "process_basic"]);
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::CommandOutputSummary;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    let loop_state = LoopState::new(1);
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({"action": "count_entries", "path": "scripts/nl_tests/fixtures/device_local/docs"}),
        },
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({"action": "count_entries", "path": "scripts/nl_tests/fixtures/device_local/logs"}),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["step_1".to_string(), "step_2".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &loop_state,
        "count direct children under two fixture directories",
        None,
        actions,
    );

    let first_args = expect_planned_call(&normalized[0], "fs_basic", "count_entries");
    assert_eq!(
        first_args.get("path").and_then(Value::as_str),
        Some("scripts/nl_tests/fixtures/device_local/docs")
    );
    let second_args = expect_planned_call(&normalized[1], "fs_basic", "count_entries");
    assert_eq!(
        second_args.get("path").and_then(Value::as_str),
        Some("scripts/nl_tests/fixtures/device_local/logs")
    );
}

#[test]
fn command_output_summary_replaces_non_recipe_mutation_with_preferred_observation() {
    let state = test_state_with_registry();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::CommandOutputSummary;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    let loop_state = LoopState::new(1);
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "write_text",
            "path": "document/nl_ops_http_repair_demo/index.html",
            "content": "VALIDATION_PASSED\n",
        }),
    }];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &loop_state,
        "repair local fixture",
        None,
        actions,
    );

    assert!(
        normalized
            .iter()
            .all(|action| !planned_call_is(action, "fs_basic", "write_text")),
        "normalized actions: {normalized:?}"
    );
    assert!(
        normalized
            .iter()
            .any(|action| planned_call_is(action, "process_basic", "ps")),
        "normalized actions: {normalized:?}"
    );
}

#[test]
fn command_output_summary_keeps_registry_non_mutating_config_preview_actions() {
    let state = test_state_with_registry();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::CommandOutputSummary;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    let actions = vec![
        AgentAction::CallTool {
            tool: "git_basic".to_string(),
            args: json!({"action": "status"}),
        },
        AgentAction::CallSkill {
            skill: "config_edit".to_string(),
            args: json!({
                "action": "plan_config_change",
                "path": "configs/config.toml",
                "field_path": "llm.selected_vendor",
                "value": "minimax",
            }),
        },
        AgentAction::CallSkill {
            skill: "config_basic".to_string(),
            args: json!({
                "action": "guard_rustclaw_config",
                "path": "configs/config.toml",
            }),
        },
    ];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "preview configs/config.toml llm.selected_vendor minimax and guard config",
        None,
        actions,
    );

    assert!(
        normalized.iter().any(|action| planned_call_is(
            action,
            "config_edit",
            "plan_config_change"
        )),
        "normalized actions: {normalized:?}"
    );
    assert!(
        normalized.iter().any(|action| planned_call_is(
            action,
            "config_basic",
            "guard_rustclaw_config"
        )),
        "normalized actions: {normalized:?}"
    );
}

#[test]
fn active_ops_apply_keeps_mutation_despite_summary_contract_hint() {
    let state = test_state_with_registry();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::CommandOutputSummary;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    let mut loop_state = LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
        target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::CurrentRepo,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        ..Default::default()
    };
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "write_text",
            "path": "document/nl_ops_http_repair_demo/index.html",
            "content": "VALIDATION_PASSED\n",
        }),
    }];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &loop_state,
        "repair local fixture",
        None,
        actions,
    );

    assert!(
        !normalized.is_empty(),
        "normalized actions should retain the mutation: {normalized:?}"
    );
    let args = expect_planned_call(&normalized[0], "fs_basic", "write_text");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some("document/nl_ops_http_repair_demo/index.html")
    );
}

#[test]
fn unavailable_skill_plan_forces_repair() {
    let state = test_state_with_enabled_skills(&["run_cmd", "read_file"]);
    let loop_state = LoopState::new(2);
    let actions = vec![AgentAction::CallSkill {
        skill: "disabled_writer".to_string(),
        args: json!({ "path": "out.txt" }),
    }];
    let route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        false,
        OutputResponseShape::Free,
    );

    assert!(should_force_actionable_plan_repair(
        &state,
        Some(&route),
        &loop_state,
        &actions
    ));
    assert_eq!(
        plan_repair_reason(&state, Some(&route), &loop_state, Some(&actions)),
        "unavailable_skill_requires_replan"
    );
}

#[test]
fn preferred_registry_skill_route_forces_repair_but_can_fallback_to_safe_run_cmd() {
    let state = test_state_with_registry();
    let loop_state = LoopState::new(2);
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": "systemctl status clawd"}),
    }];

    assert!(super::super::registry_preferred_skill_matches_route(
        &state, &route
    ));
    assert!(
        super::super::actions_use_ad_hoc_command_without_route_preferred_skill(
            &state, &route, &actions
        )
    );
    assert!(should_force_actionable_plan_repair(
        &state,
        Some(&route),
        &loop_state,
        &actions
    ));
    assert_eq!(
        plan_repair_reason(&state, Some(&route), &loop_state, Some(&actions)),
        "preferred_skill_required_for_semantic_route"
    );
    assert!(can_fallback_to_initial_plan_after_repair_failure(
        &state,
        Some(&route),
        &loop_state,
        &actions
    ));
}

#[test]
fn preferred_registry_skill_route_does_not_fallback_to_mutating_run_cmd() {
    let state = test_state_with_registry();
    let loop_state = LoopState::new(2);
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": "systemctl restart clawd"}),
    }];

    assert!(
        super::super::actions_use_ad_hoc_command_without_route_preferred_skill(
            &state, &route, &actions
        )
    );
    assert!(!can_fallback_to_initial_plan_after_repair_failure(
        &state,
        Some(&route),
        &loop_state,
        &actions
    ));
}

#[test]
fn preferred_registry_skill_route_does_not_force_repair_from_structured_tool() {
    let state = test_state_with_registry();
    let loop_state = LoopState::new(2);
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    let actions = vec![AgentAction::CallTool {
        tool: "system_basic".to_string(),
        args: json!({"action": "diagnose_runtime"}),
    }];

    assert!(super::super::registry_preferred_skill_matches_route(
        &state, &route
    ));
    assert!(
        !super::super::actions_use_ad_hoc_command_without_route_preferred_skill(
            &state, &route, &actions
        )
    );
    assert!(!should_force_actionable_plan_repair(
        &state,
        Some(&route),
        &loop_state,
        &actions
    ));
}

#[test]
fn fs_basic_directory_names_route_forces_repair_from_run_cmd() {
    let state = test_state_with_registry();
    let loop_state = LoopState::new(2);
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryNames;
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": "find . -type f -name '*.sh' | xargs dirname | sort -u"}),
    }];

    assert!(super::super::registry_preferred_skill_matches_route(
        &state, &route
    ));
    assert!(
        super::super::actions_use_ad_hoc_command_without_route_preferred_skill(
            &state, &route, &actions
        )
    );
    assert_eq!(
        plan_repair_reason(&state, Some(&route), &loop_state, Some(&actions)),
        "preferred_skill_required_for_semantic_route"
    );
}

#[test]
fn explicit_literal_run_cmd_marker_skips_preferred_skill_repair() {
    let state = test_state_with_registry();
    let loop_state = LoopState::new(2);
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::SqliteTableListing;
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({
            "command": "sqlite3 data/db-basic-contract.sqlite '.tables'",
            super::super::super::CLAWD_LITERAL_COMMAND_ARG: true
        }),
    }];

    assert!(super::super::registry_preferred_skill_matches_route(
        &state, &route
    ));
    assert!(
        !super::super::actions_use_ad_hoc_command_without_route_preferred_skill(
            &state, &route, &actions
        )
    );
    assert!(!should_force_actionable_plan_repair(
        &state,
        Some(&route),
        &loop_state,
        &actions
    ));
    assert!(can_fallback_to_initial_plan_after_repair_failure(
        &state,
        Some(&route),
        &loop_state,
        &actions
    ));
}

#[test]
fn explicit_literal_existing_run_cmd_is_marked_before_repair_checks() {
    let mut state = test_state_with_registry();
    state.policy.command_intent.execute_prefixes = vec!["执行命令 ".to_string()];
    let loop_state = LoopState::new(1);
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::SqliteTableListing;
    route.output_contract.locator_hint = "data/db-basic-contract.sqlite".to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": "sqlite3 data/db-basic-contract.sqlite '.tables'"}),
    }];

    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &loop_state,
        "执行 sqlite3 命令查询 data/db-basic-contract.sqlite 数据库中的所有表名，并返回结果。",
        Some("执行命令 sqlite3 data/db-basic-contract.sqlite \".tables\"，告诉我结果。"),
        None,
        actions,
    );

    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "run_cmd");
            assert_eq!(
                args.get(super::super::super::CLAWD_LITERAL_COMMAND_ARG)
                    .and_then(Value::as_bool),
                Some(true)
            );
        }
        other => panic!("expected run_cmd action, got {other:?}"),
    }
    assert!(!should_force_actionable_plan_repair(
        &state,
        Some(&route),
        &loop_state,
        &normalized
    ));
}

#[test]
fn explicit_literal_scalar_route_marks_failure_repairable() {
    let mut state = test_state_with_registry();
    state.policy.command_intent.execute_prefixes = vec!["执行 ".to_string()];
    let loop_state = LoopState::new(1);
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": "missing_probe --version"}),
    }];

    let normalized = normalize_planned_actions_with_original(
            &state,
            Some(&route),
            &loop_state,
            "执行 missing_probe --version；如果该命令不存在，则执行 which bash，并只返回 bash 的路径。",
            Some(
                "执行 missing_probe --version；如果该命令不存在，则执行 which bash，并只返回 bash 的路径。",
            ),
            None,
            actions,
        );

    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "run_cmd");
            assert_eq!(
                args.get(super::super::super::CLAWD_LITERAL_COMMAND_ARG)
                    .and_then(Value::as_bool),
                Some(true)
            );
            assert_eq!(
                args.get(super::super::super::CLAWD_LITERAL_FAILURE_REPAIRABLE_ARG)
                    .and_then(Value::as_bool),
                Some(true)
            );
        }
        other => panic!("expected run_cmd action, got {other:?}"),
    }
}

#[test]
fn file_paths_route_marks_missing_target_repairable() {
    let state = test_state_with_registry();
    let loop_state = LoopState::new(1);
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::FilePaths;
    let actions = vec![AgentAction::CallSkill {
        skill: "read_file".to_string(),
        args: json!({"path": "plan/missing.md"}),
    }];

    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &loop_state,
        "read missing, then find a related file",
        Some("read missing, then find a related file"),
        None,
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "read_text_range");
    assert_eq!(
        args.get(super::super::super::CLAWD_MISSING_TARGET_REPAIRABLE_ARG)
            .and_then(Value::as_bool),
        Some(true)
    );
}

#[test]
fn raw_command_output_route_does_not_force_preferred_skill_repair() {
    let state = test_state_with_registry();
    let loop_state = LoopState::new(2);
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": "systemctl status clawd"}),
    }];

    assert!(!should_force_actionable_plan_repair(
        &state,
        Some(&route),
        &loop_state,
        &actions
    ));
}

#[test]
fn repair_failure_does_not_fallback_to_unavailable_skill_plan() {
    let state = test_state_with_enabled_skills(&["run_cmd", "read_file"]);
    let loop_state = LoopState::new(2);
    let actions = vec![AgentAction::CallSkill {
        skill: "disabled_reader".to_string(),
        args: json!({ "path": "README.md" }),
    }];
    let route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        false,
        OutputResponseShape::Free,
    );

    assert!(!can_fallback_to_initial_plan_after_repair_failure(
        &state,
        Some(&route),
        &loop_state,
        &actions
    ));
}

#[test]
fn actionable_route_allows_respond_only_after_observation_exists() {
    let mut loop_state = LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    let actions = vec![AgentAction::Respond {
        content: "final answer".to_string(),
    }];
    assert!(!should_force_plan_repair(
        Some(&route_result(
            crate::AskMode::planner_execute_chat_wrapped(),
            false,
            OutputResponseShape::Free,
        )),
        &loop_state,
        &actions,
    ));
}

#[test]
fn content_evidence_route_keeps_observation_only_plan_for_observed_finalize() {
    let loop_state = LoopState::new(2);
    let actions = vec![AgentAction::CallSkill {
        skill: "read_file".to_string(),
        args: serde_json::json!({ "path": "README.md" }),
    }];
    assert!(!should_force_plan_repair(
        Some(&route_result(
            crate::AskMode::planner_execute_chat_wrapped(),
            true,
            OutputResponseShape::Free,
        )),
        &loop_state,
        &actions,
    ));
}

#[test]
fn lightweight_act_route_keeps_observation_only_plan_without_repair() {
    let loop_state = LoopState::new(2);
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: serde_json::json!({
            "action": "read_range",
            "path": "/tmp/device_local/logs/model_io.log",
            "mode": "tail",
            "n": 4
        }),
    }];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.resolved_intent = "读取 /tmp/device_local/logs/model_io.log 最后 4 行".to_string();
    route.output_contract.locator_hint = "/tmp/device_local/logs/model_io.log".to_string();
    assert!(!should_force_plan_repair(
        Some(&route),
        &loop_state,
        &actions,
    ));
}

#[test]
fn lightweight_route_rejects_unavailable_followup_skill() {
    let state = test_state_with_enabled_skills(&["read_file"]);
    let loop_state = LoopState::new(2);
    let actions = vec![
        AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: serde_json::json!({ "path": "README.md" }),
        },
        AgentAction::CallSkill {
            skill: "formatter".to_string(),
            args: serde_json::json!({ "text": "用一句话总结 {{last_output}}" }),
        },
    ];
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.route_reason = "llm_contract:generic_filename_single_read".to_string();
    route.resolved_intent = "看一下 README.md，然后一句话说它主要讲了什么".to_string();
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "README.md".to_string();
    assert!(should_force_actionable_plan_repair(
        &state,
        Some(&route),
        &loop_state,
        &actions,
    ));
    assert_eq!(
        plan_repair_reason(&state, Some(&route), &loop_state, Some(&actions)),
        "unavailable_skill_requires_replan"
    );
}

#[test]
fn clarify_followup_tail_request_does_not_rewrite_single_read_file_from_text() {
    let state = test_state();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.resolved_intent = "Continue the previous request that was waiting for clarification: 看看那个模型日志最后 5 行\nUser now provides the missing target or content: scripts/nl_tests/fixtures/device_local/logs/model_io.log".to_string();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: json!({
                "path": "scripts/nl_tests/fixtures/device_local/logs/model_io.log"
            }),
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(2),
        "scripts/nl_tests/fixtures/device_local/logs/model_io.log",
        None,
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "read_text_range");
    assert_eq!(
        args.get("path").and_then(|value| value.as_str()),
        Some("scripts/nl_tests/fixtures/device_local/logs/model_io.log")
    );
}

#[test]
fn non_range_single_read_keeps_read_file_plan() {
    let state = test_state();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.resolved_intent =
        "看看 scripts/nl_tests/fixtures/device_local/logs/model_io.log".to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "read_file".to_string(),
        args: json!({
            "path": "scripts/nl_tests/fixtures/device_local/logs/model_io.log"
        }),
    }];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(2),
        "scripts/nl_tests/fixtures/device_local/logs/model_io.log",
        None,
        actions,
    );

    assert!(
        planned_call_is(&normalized[0], "fs_basic", "read_text_range"),
        "normalized[0]={:?}",
        normalized[0]
    );
}

#[test]
fn single_target_read_file_prefers_auto_locator_file_over_stale_existing_path() {
    let state = test_state();
    let root = TempDirGuard::new("single_target_read_file");
    let stale = root.path.join("stale.log");
    let current = root.path.join("clawd.log");
    fs::write(&stale, "stale\n").expect("write stale file");
    fs::write(&current, "fresh\n").expect("write current file");
    let stale_path = stale.display().to_string();
    let current_path = current.display().to_string();

    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.resolved_intent = format!("读取 {} 的内容", current_path);
    route.output_contract.locator_hint = current_path.clone();

    let actions = vec![
        AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: json!({ "path": stale_path }),
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(2),
        "第二个的内容",
        Some(current_path.as_str()),
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "read_text_range");
    assert_eq!(
        args.get("path").and_then(|value| value.as_str()),
        Some(current_path.as_str())
    );
}

#[test]
fn single_target_read_range_prefers_auto_locator_file_over_stale_existing_path() {
    let state = test_state();
    let root = TempDirGuard::new("single_target_read_range");
    let stale = root.path.join("hello_from_manual_test.sh");
    let current = root.path.join("clawd.log");
    fs::write(&stale, "#!/bin/bash\necho stale\n").expect("write stale file");
    fs::write(&current, "line1\nline2\n").expect("write current file");
    let stale_path = stale.display().to_string();
    let current_path = current.display().to_string();

    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.resolved_intent = format!("查看 {} 最后 2 行", current_path);
    route.output_contract.locator_hint = current_path.clone();

    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "read_range",
                "path": stale_path,
                "mode": "tail",
                "n": 2
            }),
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(2),
        "第二个的最后 2 行",
        Some(current_path.as_str()),
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "read_text_range");
    assert_eq!(
        args.get("path").and_then(|value| value.as_str()),
        Some(current_path.as_str())
    );
}

#[test]
fn single_target_call_tool_system_basic_read_range_prefers_auto_locator_file() {
    let state = test_state();
    let root = TempDirGuard::new("single_target_call_tool_system_basic_read_range");
    let stale = root.path.join("clawd-dev.log");
    let current = root.path.join("clawd.codex.nltest.log");
    fs::write(&stale, "stale\n").expect("write stale file");
    fs::write(&current, "fresh\n").expect("write current file");
    let stale_path = stale.display().to_string();
    let current_path = current.display().to_string();

    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.resolved_intent = format!("active_delivery_content_target: {current_path}");
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = current_path.clone();
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;

    let actions = vec![AgentAction::CallTool {
        tool: "system_basic".to_string(),
        args: json!({
            "action": "read_range",
            "path": stale_path,
            "mode": "tail",
            "n": 1
        }),
    }];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(2),
        "read tail 1",
        Some(current_path.as_str()),
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "read_text_range");
    assert_eq!(
        args.get("path").and_then(|value| value.as_str()),
        Some(current_path.as_str())
    );
}

#[test]
fn single_target_file_read_falls_back_to_route_locator_when_auto_locator_suppressed() {
    let state = test_state();
    let root = TempDirGuard::new("single_target_route_locator_fallback");
    let stale = root.path.join("clawd-dev.log");
    let current = root.path.join("clawd.codex.nltest.log");
    fs::write(&stale, "stale\n").expect("write stale file");
    fs::write(&current, "fresh\n").expect("write current file");
    let stale_path = stale.display().to_string();
    let current_path = current.display().to_string();

    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = current_path.clone();
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;

    let actions = vec![AgentAction::CallTool {
        tool: "system_basic".to_string(),
        args: json!({
            "action": "read_range",
            "path": stale_path,
            "mode": "tail",
            "n": 1
        }),
    }];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(2),
        "read tail 1",
        None,
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "read_text_range");
    assert_eq!(
        args.get("path").and_then(|value| value.as_str()),
        Some(current_path.as_str())
    );
}

#[test]
fn route_target_file_content_plan_collapses_placeholder_read_chain() {
    let state = test_state();
    let root = TempDirGuard::new("route_target_file_content_placeholder_chain");
    let logs = root.path.join("logs");
    fs::create_dir_all(&logs).expect("create logs");
    let current = logs.join("clawd.codex.nltest.log");
    fs::write(&current, "line1\nline2\n").expect("write current file");
    let current_path = current.display().to_string();
    let logs_path = logs.display().to_string();

    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = current_path.clone();
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;

    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "list_dir",
                "path": logs_path,
                "files_only": true,
                "names_only": true
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": "{{last_output}}",
                "mode": "tail",
                "n": 1
            }),
        },
    ];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(2),
        "read",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 1);
    let args = expect_planned_call(&normalized[0], "fs_basic", "read_text_range");
    assert_eq!(
        args.get("path").and_then(|value| value.as_str()),
        Some(current_path.as_str())
    );
    assert_eq!(
        args.get("mode").and_then(|value| value.as_str()),
        Some("tail")
    );
    assert_eq!(args.get("n").and_then(|value| value.as_i64()), Some(1));
}

#[test]
fn single_target_fs_basic_read_text_range_prefers_auto_locator_file_over_stale_existing_path() {
    let state = test_state();
    let root = TempDirGuard::new("single_target_fs_basic_read_text_range");
    let stale = root.path.join("clawd-dev.log");
    let current = root.path.join("clawd.codex.nltest.log");
    fs::write(&stale, "stale\n").expect("write stale file");
    fs::write(&current, "fresh\n").expect("write current file");
    let stale_path = stale.display().to_string();
    let current_path = current.display().to_string();

    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.resolved_intent = format!("读取 {} 最后 1 行", current_path);
    route.output_contract.locator_hint = current_path.clone();

    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "read_text_range",
            "path": stale_path,
            "mode": "tail",
            "n": 1
        }),
    }];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(2),
        "读取上一文件最后 1 行",
        Some(current_path.as_str()),
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "read_text_range");
    assert_eq!(
        args.get("path").and_then(|value| value.as_str()),
        Some(current_path.as_str())
    );
}

#[test]
fn auto_locator_file_does_not_collapse_multi_read_plan() {
    let state = test_state();
    let root = TempDirGuard::new("multi_read_preserve");
    let alpha = root.path.join("alpha.log");
    let beta = root.path.join("beta.log");
    fs::write(&alpha, "alpha\n").expect("write alpha");
    fs::write(&beta, "beta\n").expect("write beta");
    let alpha_path = alpha.display().to_string();
    let beta_path = beta.display().to_string();

    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.resolved_intent = "对比两个文件".to_string();

    let actions = vec![
        AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: json!({ "path": alpha_path.clone() }),
        },
        AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: json!({ "path": beta_path.clone() }),
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(2),
        "对比 alpha 和 beta",
        Some(beta_path.as_str()),
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "read_text_range");
    assert_eq!(
        args.get("path").and_then(|value| value.as_str()),
        Some(alpha_path.as_str())
    );
    let args = expect_planned_call(&normalized[1], "fs_basic", "read_text_range");
    assert_eq!(
        args.get("path").and_then(|value| value.as_str()),
        Some(beta_path.as_str())
    );
}

#[test]
fn content_evidence_route_keeps_terminal_discussion_followup_for_planned_synthesis() {
    let loop_state = LoopState::new(2);
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "read_range",
                "path": "README.md",
                "mode": "head",
                "n": 20
            }),
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];
    let kept = strip_terminal_discussion_for_observed_finalize(
        Some(&route_result(
            crate::AskMode::planner_execute_chat_wrapped(),
            true,
            OutputResponseShape::Free,
        )),
        &loop_state,
        actions.clone(),
    );
    assert_eq!(kept.len(), 2);
    assert!(matches!(
        &kept[0],
        AgentAction::CallSkill { skill, .. } if skill == "system_basic"
    ));
    assert!(matches!(
        &kept[1],
        AgentAction::Respond { content } if content == "{{last_output}}"
    ));
}

#[test]
fn content_evidence_route_keeps_terminal_synthesize_followup_for_planned_synthesis() {
    let loop_state = LoopState::new(2);
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "read_range",
                "path": "README.md",
                "mode": "head",
                "n": 20
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
    ];
    let kept = strip_terminal_discussion_for_observed_finalize(
        Some(&route_result(
            crate::AskMode::planner_execute_chat_wrapped(),
            true,
            OutputResponseShape::Free,
        )),
        &loop_state,
        actions.clone(),
    );
    assert_eq!(kept.len(), 2);
    assert!(matches!(
        &kept[0],
        AgentAction::CallSkill { skill, .. } if skill == "system_basic"
    ));
    assert!(matches!(
        &kept[1],
        AgentAction::SynthesizeAnswer { evidence_refs } if evidence_refs == &vec!["last_output".to_string()]
    ));
}

#[test]
fn content_evidence_route_keeps_multi_evidence_synthesize_followup() {
    let loop_state = LoopState::new(2);
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "read_range",
                "path": "service_notes.md",
                "mode": "head",
                "n": 20
            }),
        },
        AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: serde_json::json!({ "path": "README.md" }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["s1".to_string(), "s2".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];
    let kept = strip_terminal_discussion_for_observed_finalize(
        Some(&route_result(
            crate::AskMode::planner_execute_chat_wrapped(),
            true,
            OutputResponseShape::Free,
        )),
        &loop_state,
        actions.clone(),
    );
    assert_eq!(kept.len(), 4);
    assert!(matches!(
        &kept[0],
        AgentAction::CallSkill { skill, .. } if skill == "system_basic"
    ));
    assert!(matches!(
        &kept[1],
        AgentAction::CallSkill { skill, .. } if skill == "read_file"
    ));
    assert!(matches!(
        &kept[2],
        AgentAction::SynthesizeAnswer { evidence_refs }
            if evidence_refs == &vec!["s1".to_string(), "s2".to_string()]
    ));
    assert!(matches!(
        &kept[3],
        AgentAction::Respond { content } if content == "{{last_output}}"
    ));
}

#[test]
fn recent_scalar_pair_strips_terminal_synthesis_for_runtime_finalizer() {
    let loop_state = LoopState::new(2);
    let actions = vec![
        AgentAction::CallTool {
            tool: "config_basic".to_string(),
            args: serde_json::json!({
                "action": "read_field",
                "path": "UI/package.json",
                "field_path": "name"
            }),
        },
        AgentAction::CallTool {
            tool: "config_basic".to_string(),
            args: serde_json::json!({
                "action": "read_field",
                "path": "crates/clawd/Cargo.toml",
                "field_path": "package.name"
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["step_1".to_string(), "step_2".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RecentScalarEqualityCheck;

    let stripped =
        strip_terminal_discussion_for_observed_finalize(Some(&route), &loop_state, actions);

    assert_eq!(stripped.len(), 2);
    assert!(matches!(
        &stripped[0],
        AgentAction::CallTool { tool, args }
            if tool == "config_basic"
                && args.get("action").and_then(Value::as_str) == Some("read_field")
    ));
    assert!(matches!(
        &stripped[1],
        AgentAction::CallTool { tool, args }
            if tool == "config_basic"
                && args.get("action").and_then(Value::as_str) == Some("read_field")
    ));
    assert!(!should_force_actionable_plan_repair(
        &test_state(),
        Some(&route),
        &loop_state,
        &stripped
    ));
}

use super::*;

#[test]
fn session_alias_delivery_rewrites_stale_stat_path_to_route_locator() {
    let mut route = delivery_route_result();
    route.output_contract.delivery_intent = OutputDeliveryIntent::FileSingle;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/current/missing.md".to_string();
    route.route_reason = "session_alias_locator_prebound_from_current_request".to_string();
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "stat_paths",
                "paths": ["/tmp/old/service_notes.md"]
            }),
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let loop_state = LoopState::default();
    let normalized = rewrite_session_alias_delivery_observations_to_route_locator(
        Some(&route),
        &loop_state,
        actions,
    );

    let Some((_, args)) = planned_call(&normalized[0]) else {
        panic!("expected call");
    };
    assert_eq!(
        args.pointer("/paths/0").and_then(Value::as_str),
        Some("/tmp/current/missing.md")
    );
}

#[test]
fn active_bound_target_rewrites_matching_basename_without_route_prebind_marker() {
    let mut route = route_result(
        crate::AskMode::direct_answer(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "test_bundle.zip".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::ArchiveList;
    route.output_contract.requires_content_evidence = true;
    let mut loop_state = LoopState::new(2);
    loop_state.output_vars.insert(
        "active_bound_targets".to_string(),
        json!(["scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip"]).to_string(),
    );
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "stat_paths",
            "paths": ["test_bundle.zip"]
        }),
    }];

    let normalized = rewrite_active_bound_target_observations_to_matching_locator_hint(
        Some(&route),
        &loop_state,
        actions,
    );

    assert_eq!(route.output_contract.locator_hint, "test_bundle.zip");
    let Some((_, args)) = planned_call(&normalized[0]) else {
        panic!("expected call");
    };
    assert_eq!(
        args.pointer("/paths/0").and_then(Value::as_str),
        Some("scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip")
    );
}

#[test]
fn session_alias_delivery_rewrites_from_loop_required_alias_target_without_route_marker() {
    let mut route = delivery_route_result();
    route.output_contract.delivery_intent = OutputDeliveryIntent::FileSingle;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    let loop_state = loop_state_with_required_session_alias_targets(&["/tmp/current/alias.md"]);
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "stat_paths",
                "paths": ["/tmp/old/service_notes.md"]
            }),
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = rewrite_session_alias_delivery_observations_to_route_locator(
        Some(&route),
        &loop_state,
        actions,
    );

    let Some((_, args)) = planned_call(&normalized[0]) else {
        panic!("expected call");
    };
    assert_eq!(
        args.pointer("/paths/0").and_then(Value::as_str),
        Some("/tmp/current/alias.md")
    );
}

#[test]
fn multi_session_alias_target_plan_requires_all_targets_before_execution() {
    let loop_state = loop_state_with_required_session_alias_targets(&[
        "/tmp/docs/archive",
        "/tmp/docs/release_checklist.md",
    ]);
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryNames;
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "list_dir",
            "path": "/tmp/docs/archive"
        }),
    }];

    assert!(should_force_plan_repair(
        Some(&route),
        &loop_state,
        &actions
    ));
    assert_eq!(
        repair_reason(Some(&route), &loop_state, Some(&actions)),
        "current_request_mentions_multiple_session_alias_targets_but_plan_omits_target"
    );
    assert!(!can_fallback_to_initial_plan_after_repair_failure(
        &test_state(),
        Some(&route),
        &loop_state,
        &actions
    ));
}

#[test]
fn multi_session_alias_target_plan_accepts_actions_covering_all_targets() {
    let loop_state = loop_state_with_required_session_alias_targets(&[
        "/tmp/docs/archive",
        "/tmp/docs/release_checklist.md",
    ]);
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryNames;
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "list_dir",
                "path": "/tmp/docs/archive"
            }),
        },
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": "/tmp/docs/release_checklist.md",
                "max_lines": 20
            }),
        },
    ];

    assert!(!should_force_plan_repair(
        Some(&route),
        &loop_state,
        &actions
    ));
}

#[test]
fn normalizer_completes_missing_session_alias_file_target_observation() {
    let tmp = TempDirGuard::new("session_alias_complete_missing_file");
    let archive_dir = tmp.path.join("docs/archive");
    fs::create_dir_all(&archive_dir).expect("create archive");
    fs::write(archive_dir.join("README.txt"), "archive notes\n").expect("write archive file");
    let checklist = tmp.path.join("docs/release_checklist.md");
    fs::write(&checklist, "verify config, migrations, and logs\n").expect("write checklist");
    let mut state = test_state_with_registry();
    state.skill_rt.workspace_root = tmp.path.clone();
    let loop_state = loop_state_with_required_session_alias_targets(&[
        archive_dir.to_string_lossy().as_ref(),
        checklist.to_string_lossy().as_ref(),
    ]);
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryEntryGroups;
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "list_dir",
            "path": archive_dir.to_string_lossy()
        }),
    }];

    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &loop_state,
        "multi alias request",
        None,
        None,
        actions,
    );

    assert!(normalized.iter().any(|action| {
        action_capability_and_action(action, "fs_basic", "list_dir").is_some_and(|args| {
            args.get("path").and_then(Value::as_str) == Some(archive_dir.to_string_lossy().as_ref())
        })
    }));
    assert!(normalized.iter().any(|action| {
        action_capability_and_action(action, "fs_basic", "read_text_range").is_some_and(|args| {
            args.get("path").and_then(Value::as_str) == Some(checklist.to_string_lossy().as_ref())
                && args.get("mode").and_then(Value::as_str) == Some("head")
        })
    }));
    assert!(normalized.iter().any(|action| {
        matches!(
            action,
            AgentAction::SynthesizeAnswer { evidence_refs } if evidence_refs.len() >= 2
        )
    }));
    assert!(matches!(
        normalized.last(),
        Some(AgentAction::Respond { content }) if content == "{{last_output}}"
    ));
}

#[test]
fn normalizer_recovers_session_alias_targets_from_plan_context_alias_block() {
    let tmp = TempDirGuard::new("session_alias_context_recovery");
    let archive_dir = tmp.path.join("docs/archive");
    fs::create_dir_all(&archive_dir).expect("create archive");
    let archive_readme = archive_dir.join("README.txt");
    fs::write(&archive_readme, "archive notes\n").expect("write archive readme");
    let checklist = tmp.path.join("docs/release_checklist.md");
    fs::write(&checklist, "verify config, migrations, and logs\n").expect("write checklist");
    let mut state = test_state_with_registry();
    state.skill_rt.workspace_root = tmp.path.clone();
    let loop_state = LoopState::new(2);
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryPurposeSummary;
    let plan_context = format!(
        "resolved_prompt=列取甲目录内容并摘要乙文件核心提醒\n\n\
### SESSION_ALIAS_BINDINGS\n\
- alias: 甲目录\n\
  target: {}\n\
- alias: 乙文件\n\
  target: {}\n",
        archive_dir.display(),
        checklist.display()
    );
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "list_dir",
                "path": archive_dir.to_string_lossy()
            }),
        },
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": archive_readme.to_string_lossy(),
                "mode": "head",
                "n": 40
            }),
        },
    ];

    let normalized = normalize_planned_actions_with_original_and_context(
        &state,
        Some(&route),
        &loop_state,
        "列一下甲目录里的名字，再顺手说乙文件主要在提醒什么",
        Some("列一下甲目录里的名字，再顺手说乙文件主要在提醒什么"),
        Some(&plan_context),
        None,
        actions,
    );

    assert!(normalized.iter().any(|action| {
        action_capability_and_action(action, "fs_basic", "read_text_range").is_some_and(|args| {
            args.get("path").and_then(Value::as_str) == Some(checklist.to_string_lossy().as_ref())
        })
    }));
}

#[test]
fn normalizer_recovers_session_alias_targets_from_boundary_observation_block() {
    let tmp = TempDirGuard::new("session_alias_boundary_observation_recovery");
    let archive_dir = tmp.path.join("docs/archive");
    fs::create_dir_all(&archive_dir).expect("create archive");
    let archive_readme = archive_dir.join("README.txt");
    fs::write(&archive_readme, "archive notes\n").expect("write archive readme");
    let checklist = tmp.path.join("docs/release_checklist.md");
    fs::write(&checklist, "verify config, migrations, and logs\n").expect("write checklist");
    let mut state = test_state_with_registry();
    state.skill_rt.workspace_root = tmp.path.clone();
    let loop_state = LoopState::new(2);
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryPurposeSummary;
    let observation = json!({
        "kind": "agent_loop_boundary_observations",
        "schema_version": 1,
        "session_alias_bindings": [
            {"alias": "甲目录", "target": archive_dir.to_string_lossy()},
            {"alias": "乙文件", "target": checklist.to_string_lossy()}
        ]
    });
    let plan_context = format!(
        "resolved_prompt=列取甲目录内容并摘要乙文件核心提醒\n\n\
### AGENT_LOOP_BOUNDARY_OBSERVATIONS\n{}\n### END_AGENT_LOOP_BOUNDARY_OBSERVATIONS\n",
        observation
    );
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "list_dir",
                "path": archive_dir.to_string_lossy()
            }),
        },
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": archive_readme.to_string_lossy(),
                "mode": "head",
                "n": 40
            }),
        },
    ];

    let normalized = normalize_planned_actions_with_original_and_context(
        &state,
        Some(&route),
        &loop_state,
        "列一下甲目录里的名字，再顺手说乙文件主要在提醒什么",
        Some("列一下甲目录里的名字，再顺手说乙文件主要在提醒什么"),
        Some(&plan_context),
        None,
        actions,
    );

    assert!(normalized.iter().any(|action| {
        action_capability_and_action(action, "fs_basic", "read_text_range").is_some_and(|args| {
            args.get("path").and_then(Value::as_str) == Some(checklist.to_string_lossy().as_ref())
        })
    }));
}

#[test]
fn normalizer_recovers_session_alias_targets_from_goal_alias_block() {
    let tmp = TempDirGuard::new("session_alias_goal_context_recovery");
    let archive_dir = tmp.path.join("docs/archive");
    fs::create_dir_all(&archive_dir).expect("create archive");
    let archive_readme = archive_dir.join("README.txt");
    fs::write(&archive_readme, "archive notes\n").expect("write archive readme");
    let checklist = tmp.path.join("docs/release_checklist.md");
    fs::write(&checklist, "verify config, migrations, and logs\n").expect("write checklist");
    let mut state = test_state_with_registry();
    state.skill_rt.workspace_root = tmp.path.clone();
    let loop_state = LoopState::new(2);
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryPurposeSummary;
    let goal = format!(
        "resolved_prompt=列取甲目录内容并摘要乙文件核心提醒\n\n\
### SESSION_ALIAS_BINDINGS\n\
- alias: 甲目录\n\
  target: {}\n\
- alias: 乙文件\n\
  target: {}\n",
        archive_dir.display(),
        checklist.display()
    );
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "list_dir",
                "path": archive_dir.to_string_lossy()
            }),
        },
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": archive_readme.to_string_lossy(),
                "mode": "head",
                "n": 40
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["step_1".to_string(), "step_2".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = normalize_planned_actions_with_original_and_context(
        &state,
        Some(&route),
        &loop_state,
        &goal,
        Some("列一下甲目录里的名字，再顺手说乙文件主要在提醒什么"),
        None,
        None,
        actions,
    );

    assert!(normalized.iter().any(|action| {
        action_capability_and_action(action, "fs_basic", "read_text_range").is_some_and(|args| {
            args.get("path").and_then(Value::as_str) == Some(checklist.to_string_lossy().as_ref())
        })
    }));
    assert!(normalized.iter().any(|action| {
        matches!(
            action,
            AgentAction::SynthesizeAnswer { evidence_refs } if evidence_refs.len() >= 2
        )
    }));
}

#[test]
fn actionable_route_repairs_respond_only_plan_before_any_observation() {
    let loop_state = LoopState::new(2);
    let actions = vec![AgentAction::Respond {
        content: "final answer".to_string(),
    }];
    assert!(should_force_plan_repair(
        Some(&route_result(
            crate::AskMode::planner_execute_with_chat_finalizer(),
            false,
            OutputResponseShape::Free,
        )),
        &loop_state,
        &actions,
    ));
}

#[test]
fn pure_chat_agent_loop_submode_allows_respond_only_plan_before_observation() {
    let loop_state = LoopState::new(2);
    let actions = vec![AgentAction::Respond {
        content: "final answer".to_string(),
    }];
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        false,
        OutputResponseShape::OneSentence,
    );
    route.route_reason = "pure_chat_agent_loop_submode".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();

    assert!(!should_force_plan_repair(
        Some(&route),
        &loop_state,
        &actions,
    ));
}

#[test]
fn tool_discovery_route_allows_context_only_respond_plan() {
    let loop_state = LoopState::new(2);
    let actions = vec![AgentAction::Respond {
        content: "capability inventory".to_string(),
    }];
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        false,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ToolDiscovery;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();

    assert!(!should_force_plan_repair(
        Some(&route),
        &loop_state,
        &actions,
    ));
}

#[test]
fn plain_act_path_action_rejects_readonly_file_plan_before_execution() {
    let loop_state = LoopState::new(1);
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "document/nl_tool200/group_02/memo.txt".to_string();
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": "/home/guagua/rustclaw/document/nl_tool200/group_02/memo.txt",
                "mode": "head",
                "n": 120
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["step_1".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    assert!(should_force_plan_repair(
        Some(&route),
        &loop_state,
        &actions
    ));
    assert_eq!(
        repair_reason(Some(&route), &loop_state, Some(&actions)),
        "plain_act_file_action_requires_non_readonly_plan"
    );
}

#[tokio::test]
async fn active_task_append_current_locator_reaches_planner_path() {
    let state = test_state();
    let task = ClaimedTask {
        task_id: "active-task-append-plan-round".to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: json!({ "text": "append beta to the active file" }).to_string(),
    };
    let loop_state = LoopState::new(1);
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    let analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::TaskAppend),
        target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
        should_interrupt_active_run: false,
        attachment_processing_required: false,
        state_patch: Some(json!({
            "deictic_reference": {"target": "current_turn_locator"},
            "required_content_literals": ["beta"]
        })),
    };
    let policy = super::super::super::support::load_agent_loop_guard_policy(&state);

    let err = super::super::plan_round_actions(
        &state,
        &task,
        "append beta to the active file",
        "append beta to the active file",
        &policy,
        &loop_state,
        Some(&analysis),
        None,
        Some(&route),
        Some("/home/guagua/rustclaw/document/nl_tool200/group_02/memo.txt"),
    )
    .await
    .expect_err("active-task append should reach planner instead of pre-LLM append plan");
    assert!(
        err.contains("required prompt missing"),
        "expected missing planner prompt after deterministic shortcut removal, got: {err}"
    );
    assert!(
        !err.contains("plan_deterministic_active_task_append_current_locator"),
        "old append deterministic fallback leaked into planner error: {err}"
    );
}

#[test]
fn execute_route_without_content_evidence_rejects_doc_parse_only_file_plan() {
    let loop_state = LoopState::new(1);
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        false,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "document/nl_tool200/group_02/memo.txt".to_string();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "doc_parse".to_string(),
            args: json!({
                "action": "parse_doc",
                "path": "/home/guagua/rustclaw/document/nl_tool200/group_02/memo.txt",
                "mode": "auto"
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    assert!(should_force_plan_repair(
        Some(&route),
        &loop_state,
        &actions
    ));
    assert_eq!(
        repair_reason(Some(&route), &loop_state, Some(&actions)),
        "execute_route_requires_non_readonly_file_plan"
    );
    assert!(!can_fallback_to_initial_plan_after_repair_failure(
        &test_state(),
        Some(&route),
        &loop_state,
        &actions
    ));
}

#[test]
fn existing_observed_synthesis_read_only_file_plan_does_not_force_repair() {
    let loop_state = LoopState::new(1);
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        false,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/home/guagua/rustclaw/logs/act_plan.log".to_string();
    route.route_reason = "existing_observed_context_synthesis".to_string();
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": "/home/guagua/rustclaw/logs/act_plan.log",
                "mode": "tail",
                "n": 3
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    assert!(!should_force_plan_repair(
        Some(&route),
        &loop_state,
        &actions
    ));
    assert!(can_fallback_to_initial_plan_after_repair_failure(
        &test_state(),
        Some(&route),
        &loop_state,
        &actions
    ));
}

#[test]
fn active_anchor_detached_read_only_plan_does_not_force_repair() {
    let loop_state = LoopState::new(1);
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        false,
        OutputResponseShape::OneSentence,
    );
    route.route_reason = "active_task_scope_refinement_detached_from_structured_anchor".to_string();
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": "/home/guagua/rustclaw/logs/clawd-codex-current.log",
                "mode": "tail",
                "n": 2
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    assert!(!should_force_plan_repair(
        Some(&route),
        &loop_state,
        &actions
    ));
    assert!(can_fallback_to_initial_plan_after_repair_failure(
        &test_state(),
        Some(&route),
        &loop_state,
        &actions
    ));
}

#[test]
fn content_evidence_route_accepts_doc_parse_file_plan() {
    let loop_state = LoopState::new(1);
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "README.md".to_string();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "doc_parse".to_string(),
            args: json!({
                "action": "parse_doc",
                "path": "/home/guagua/rustclaw/README.md",
                "mode": "auto"
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    assert!(!should_force_plan_repair(
        Some(&route),
        &loop_state,
        &actions
    ));
}

#[test]
fn content_evidence_route_repairs_respond_only_plan_even_in_chat_mode() {
    let loop_state = LoopState::new(2);
    let actions = vec![AgentAction::Respond {
        content: "guessed answer".to_string(),
    }];
    assert!(should_force_plan_repair(
        Some(&route_result(
            crate::AskMode::direct_answer(),
            true,
            OutputResponseShape::Free,
        )),
        &loop_state,
        &actions,
    ));
}

#[test]
fn content_evidence_route_repairs_synthesize_only_plan_before_any_observation() {
    let loop_state = LoopState::new(2);
    let actions = vec![
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];
    let route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );

    assert!(should_force_plan_repair(
        Some(&route),
        &loop_state,
        &actions,
    ));
    assert_eq!(
        repair_reason(Some(&route), &loop_state, Some(&actions)),
        "non_actionable_plan_for_current_route"
    );
}

#[test]
fn content_evidence_route_repairs_locator_only_observation_plan() {
    let loop_state = LoopState::new(2);
    let actions = vec![AgentAction::CallSkill {
        skill: "fs_search".to_string(),
        args: json!({
            "action": "find_name",
            "pattern": "crates/clawd/src/prompt_utils.rs",
        }),
    }];
    let route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );

    assert!(should_force_plan_repair(
        Some(&route),
        &loop_state,
        &actions,
    ));
    assert_eq!(
        repair_reason(Some(&route), &loop_state, Some(&actions)),
        "content_evidence_requires_content_observation"
    );
}

#[test]
fn content_evidence_route_accepts_structured_listing_terminal_plan() {
    let loop_state = LoopState::new(2);
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "find_entries",
                "root": "/workspace",
                "target_kind": "file",
                "name_pattern": "README",
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];
    let route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );

    assert!(!should_force_plan_repair(
        Some(&route),
        &loop_state,
        &actions,
    ));
    assert_ne!(
        repair_reason(Some(&route), &loop_state, Some(&actions)),
        "content_evidence_requires_content_observation"
    );
}

#[test]
fn existence_route_accepts_stat_paths_synthesized_metadata_evidence() {
    let loop_state = LoopState::new(2);
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "stat_paths",
                "paths": ["README.md", "README.zh-CN.md", "Cargo.toml"]
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["step_1".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;

    assert!(!should_force_plan_repair(
        Some(&route),
        &loop_state,
        &actions,
    ));
    assert_ne!(
        repair_reason(Some(&route), &loop_state, Some(&actions)),
        "content_evidence_requires_content_observation"
    );
}

#[test]
fn existence_route_accepts_observation_only_stat_paths_for_runtime_finalizer() {
    let loop_state = LoopState::new(2);
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "stat_paths",
            "paths": ["/workspace/README.md"]
        }),
    }];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        false,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;

    assert!(!should_force_plan_repair(
        Some(&route),
        &loop_state,
        &actions,
    ));
    assert_ne!(
        repair_reason(Some(&route), &loop_state, Some(&actions)),
        "plan_missing_terminal_user_answer"
    );
}

#[test]
fn existence_route_accepts_observation_only_stat_paths_even_when_content_evidence_required() {
    let loop_state = LoopState::new(1);
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "stat_paths",
            "paths": ["/workspace/missing.txt"],
            "include_missing": true
        }),
    }];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;

    assert!(!should_force_plan_repair(
        Some(&route),
        &loop_state,
        &actions,
    ));
    assert_ne!(
        repair_reason(Some(&route), &loop_state, Some(&actions)),
        "content_evidence_requires_content_observation"
    );
}

#[test]
fn generic_path_route_accepts_stat_paths_synthesized_metadata_evidence() {
    let loop_state = LoopState::new(1);
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "stat_paths",
                "paths": ["scripts/nl_tests/fixtures/device_local/docs/missing.md"],
                "include_missing": true
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
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/docs/missing.md".to_string();

    assert!(!should_force_plan_repair(
        Some(&route),
        &loop_state,
        &actions,
    ));
    assert_ne!(
        repair_reason(Some(&route), &loop_state, Some(&actions)),
        "content_evidence_requires_content_observation"
    );
}

#[test]
fn directory_names_route_accepts_fs_basic_find_entries_evidence() {
    let loop_state = LoopState::new(2);
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "find_entries",
                "root": "/workspace",
                "target_kind": "file",
                "ext_filter": "sh",
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["step_1".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryNames;

    assert!(!should_force_plan_repair(
        Some(&route),
        &loop_state,
        &actions,
    ));
    assert_ne!(
        repair_reason(Some(&route), &loop_state, Some(&actions)),
        "content_evidence_requires_content_observation"
    );
}

#[test]
fn content_evidence_route_accepts_scoped_grep_observation_plan() {
    let loop_state = LoopState::new(2);
    let actions = vec![AgentAction::CallSkill {
        skill: "fs_search".to_string(),
        args: json!({
            "action": "grep_text",
            "path": "crates/clawd/src/prompt_utils.rs",
            "query": "run_cmd",
        }),
    }];
    let route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );

    assert!(!should_force_plan_repair(
        Some(&route),
        &loop_state,
        &actions,
    ));
}

#[test]
fn content_presence_route_accepts_text_read_observation_plan() {
    let loop_state = LoopState::new(2);
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md",
                "mode": "head",
                "n": 120
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
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ContentPresenceCheck;

    assert!(
        !should_force_plan_repair(Some(&route), &loop_state, &actions),
        "unexpected repair reason: {}",
        repair_reason(Some(&route), &loop_state, Some(&actions))
    );
}

#[test]
fn workspace_synthesis_respond_only_plan_gets_default_evidence_actions() {
    let loop_state = LoopState::new(2);
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::WorkspaceProjectSummary;
    let actions = vec![AgentAction::Respond {
        content: "guessed release note".to_string(),
    }];

    let normalized = normalize_planned_actions(
        &test_state(),
        Some(&route),
        &loop_state,
        "Write a short release note for RustClaw.",
        None,
        actions,
    );
    assert!(normalized.iter().any(|action| matches!(
        action,
        AgentAction::CallSkill { skill, args }
            if skill == "git_basic"
                && args.get("action").and_then(|value| value.as_str()) == Some("log")
    )));
    assert!(normalized.iter().any(|action| matches!(
        action,
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "fs_basic"
                && args.get("action").and_then(|value| value.as_str())
                    == Some("read_text_range")
                && args.get("path").and_then(|value| value.as_str()) == Some("README.md")
    )));
    assert!(!should_force_actionable_plan_repair(
        &test_state(),
        Some(&route),
        &loop_state,
        &normalized
    ));
}

#[test]
fn workspace_synthesis_plan_adds_missing_text_evidence_and_synthesizes_all_steps() {
    let loop_state = LoopState::new(2);
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::WorkspaceProjectSummary;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({"action":"tree_summary","path":"."}),
        },
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action":"extract_fields",
                "path":"Cargo.toml",
                "field_paths":["workspace.package.version"]
            }),
        },
        AgentAction::Respond {
            content: "# Release\nSee README.md\n- guessed from Cargo.toml".to_string(),
        },
    ];

    let normalized = normalize_planned_actions(
        &test_state(),
        Some(&route),
        &loop_state,
        "Write a short release note for RustClaw.",
        None,
        actions,
    );
    assert!(normalized.iter().any(|action| matches!(
        action,
        AgentAction::CallSkill { skill, args }
            if skill == "git_basic"
                && args.get("action").and_then(|value| value.as_str()) == Some("log")
    )));
    assert!(normalized.iter().any(|action| matches!(
        action,
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "fs_basic"
                && args.get("action").and_then(|value| value.as_str())
                    == Some("read_text_range")
                && args.get("path").and_then(|value| value.as_str()) == Some("README.md")
    )));
    let synth_refs = normalized.iter().find_map(|action| match action {
        AgentAction::SynthesizeAnswer { evidence_refs } => Some(evidence_refs),
        _ => None,
    });
    assert_eq!(
        synth_refs,
        Some(&vec![
            "step_1".to_string(),
            "step_2".to_string(),
            "step_3".to_string(),
            "step_4".to_string(),
        ])
    );
    assert!(!should_force_actionable_plan_repair(
        &test_state(),
        Some(&route),
        &loop_state,
        &normalized
    ));
}

#[test]
fn workspace_discovery_only_plan_waits_for_text_evidence_before_synthesis() {
    let loop_state = LoopState::new(2);
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({"action": "workspace_glance", "path": ".", "max_entries": 30}),
        },
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({"action": "find_path", "name": "README.md", "target_kind": "file"}),
        },
    ];

    let normalized = normalize_planned_actions(
        &test_state(),
        Some(&route),
        &loop_state,
        "Write a deployment note for the current project.",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 2);
    assert!(normalized.iter().all(|action| {
        !matches!(
            action,
            AgentAction::SynthesizeAnswer { .. } | AgentAction::Respond { .. }
        )
    }));
}

#[test]
fn workspace_text_read_observation_can_append_synthesis() {
    let loop_state = LoopState::new(2);
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({"action": "read_range", "path": "README.md", "mode": "head", "n": 40}),
    }];

    let normalized = normalize_planned_actions(
        &test_state(),
        Some(&route),
        &loop_state,
        "Write a deployment note for the current project.",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 3);
    assert!(matches!(
        &normalized[1],
        AgentAction::SynthesizeAnswer { evidence_refs } if evidence_refs == &vec!["step_1".to_string()]
    ));
    assert!(matches!(
        &normalized[2],
        AgentAction::Respond { content } if content == "{{last_output}}"
    ));
}

#[test]
fn workspace_default_evidence_does_not_expand_mixed_last_output_answer() {
    let loop_state = LoopState::new(2);
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({"command": "pwd"}),
        },
        AgentAction::Respond {
            content: "{{last_output}} 是当前工作目录，通常对应正在操作的项目根目录。".to_string(),
        },
    ];

    let normalized = normalize_planned_actions(
        &test_state(),
        Some(&route),
        &loop_state,
        "执行 pwd，然后用一句话解释这个路径大概是什么",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 3);
    assert!(normalized.iter().all(|action| {
        !matches!(
            action,
            AgentAction::CallSkill { skill, .. }
                if skill == "git_basic" || skill == "system_basic"
        )
    }));
    assert!(matches!(
        &normalized[1],
        AgentAction::SynthesizeAnswer { evidence_refs }
            if evidence_refs == &vec!["step_1".to_string()]
                || evidence_refs == &vec!["last_output".to_string()]
    ));
    assert!(matches!(
        &normalized[2],
        AgentAction::Respond { content } if content == "{{last_output}}"
    ));
}

#[test]
fn listing_grounded_workspace_synthesis_does_not_expand_default_text_evidence() {
    let loop_state = LoopState::new(2);
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({"action": "inventory_dir", "path": ".", "names_only": true}),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["step_1".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = normalize_planned_actions(
        &test_state(),
        Some(&route),
        &loop_state,
        "List the current directory, then answer from that listing.",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 3);
    assert!(!normalized.iter().any(|action| {
        matches!(action, AgentAction::CallSkill { skill, .. } if skill == "git_basic")
            || matches!(
                action,
                AgentAction::CallSkill { skill, args }
                    if skill == "system_basic"
                        && args.get("action").and_then(Value::as_str) == Some("read_range")
                        && args.get("path").and_then(Value::as_str) == Some("README.md")
            )
    }));
    assert!(matches!(
        &normalized[1],
        AgentAction::SynthesizeAnswer { evidence_refs }
            if evidence_refs == &vec!["step_1".to_string()]
    ));
}

#[test]
fn workspace_default_evidence_does_not_expand_structured_count_answer() {
    let loop_state = LoopState::new(2);
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({"action": "count_inventory", "path": "crates"}),
        },
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({"action": "count_inventory", "path": "crates/skills"}),
        },
        AgentAction::Respond {
            content: "{{s1.output}} | {{s2.output}}".to_string(),
        },
    ];

    let normalized = normalize_planned_actions(
        &test_state(),
        Some(&route),
        &loop_state,
        "count two directories and explain the layout",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 4);
    assert!(normalized.iter().all(|action| {
        !matches!(
            action,
            AgentAction::CallSkill { skill, .. } if skill == "git_basic"
        )
    }));
    assert!(matches!(
        &normalized[2],
        AgentAction::SynthesizeAnswer { evidence_refs }
            if evidence_refs == &vec!["step_1".to_string(), "step_2".to_string()]
    ));
}

#[test]
fn workspace_default_evidence_does_not_expand_single_structured_count_answer() {
    let loop_state = LoopState::new(1);
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "count_inventory",
                "path": ".",
                "kind_filter": "file",
                "recursive": false,
                "include_hidden": false
            }),
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = normalize_planned_actions(
        &test_state(),
        Some(&route),
        &loop_state,
        "数一下当前目录一级有多少个普通文件，只告诉我数字和一句解释",
        None,
        actions,
    );

    assert!(matches!(
        &normalized[0],
        AgentAction::CallTool { tool: skill, args }
            if skill == "fs_basic"
        && args.get("action").and_then(Value::as_str) == Some("count_entries")
    ));
    assert!(!normalized.iter().any(|action| {
        matches!(action, AgentAction::CallSkill { skill, .. } if skill == "git_basic")
            || matches!(
                action,
                AgentAction::CallSkill { skill, args }
                    if skill == "system_basic"
                        && args.get("action").and_then(Value::as_str) == Some("read_range")
            )
    }));
}

#[test]
fn compound_listing_and_content_synthesis_refs_include_both_observations() {
    let loop_state = LoopState::new(1);
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ExcerptKindJudgment;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md".to_string();
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "list_dir",
                "path": "scripts/nl_tests/fixtures/device_local/docs",
                "names_only": true
            }),
        },
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md",
                "mode": "head",
                "n": 15
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = normalize_planned_actions(
        &test_state(),
        Some(&route),
        &loop_state,
        "list file names, read one file, then classify it",
        None,
        actions,
    );

    assert!(matches!(
        &normalized[2],
        AgentAction::SynthesizeAnswer { evidence_refs }
            if evidence_refs == &vec!["step_1".to_string(), "step_2".to_string()]
    ));
}

#[test]
fn content_excerpt_summary_listing_and_content_synthesis_refs_include_both_observations() {
    let loop_state = LoopState::new(1);
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "scripts/nl_tests/fixtures/device_local/docs".to_string();
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "list_dir",
                "path": "scripts/nl_tests/fixtures/device_local/docs",
                "names_only": true
            }),
        },
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md",
                "mode": "head",
                "n": 20
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = normalize_planned_actions(
        &test_state(),
        Some(&route),
        &loop_state,
        "list file names, read one file, then classify it",
        None,
        actions,
    );

    assert!(matches!(
        &normalized[2],
        AgentAction::SynthesizeAnswer { evidence_refs }
            if evidence_refs == &vec!["step_1".to_string(), "step_2".to_string()]
    ));
}

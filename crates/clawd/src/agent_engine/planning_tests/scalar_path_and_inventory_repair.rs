use super::*;

#[test]
fn recent_scalar_pair_normalization_strips_terminal_synthesis_for_runtime_finalizer() {
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
        crate::AskMode::act_with_chat_finalizer(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RecentScalarEqualityCheck;
    route.resolved_intent =
        "读取 UI/package.json 里的 name，再读取 crates/clawd/Cargo.toml 里的 package.name"
            .to_string();

    let normalized = normalize_planned_actions(
        &test_state(),
        Some(&route),
        &loop_state,
        &route.resolved_intent,
        Some("/home/guagua/rustclaw/UI/package.json"),
        actions,
    );

    assert_eq!(normalized.len(), 2);
    assert!(normalized.iter().all(
        |action| matches!(action, AgentAction::CallTool { tool, args }
            if tool == "config_basic"
                && args.get("action").and_then(Value::as_str) == Some("read_field"))
    ));
}

#[test]
fn recent_scalar_pair_observation_only_normalization_does_not_append_synthesis() {
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
    ];
    let mut route = route_result(
        crate::AskMode::act_with_chat_finalizer(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RecentScalarEqualityCheck;
    route.resolved_intent =
        "读取 UI/package.json 里的 name，再读取 crates/clawd/Cargo.toml 里的 package.name"
            .to_string();

    let normalized = normalize_planned_actions(
        &test_state(),
        Some(&route),
        &loop_state,
        &route.resolved_intent,
        Some("/home/guagua/rustclaw/UI/package.json"),
        actions,
    );

    assert_eq!(normalized.len(), 2);
    assert!(normalized.iter().all(
        |action| matches!(action, AgentAction::CallTool { tool, args }
            if tool == "config_basic"
                && args.get("action").and_then(Value::as_str) == Some("read_field"))
    ));
}

#[test]
fn scalar_path_observation_strips_guessed_terminal_respond() {
    let loop_state = LoopState::new(1);
    let mut route = route_result(
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    route.output_contract.delivery_required = false;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "path_batch_facts",
                "paths": ["/workspace/stem_unique/abcd"],
                "include_missing": true
            }),
        },
        AgentAction::Respond {
            content: "/workspace/stem_unique/abcd".to_string(),
        },
    ];

    let kept =
        strip_terminal_discussion_for_scalar_path_observation(Some(&route), &loop_state, actions);
    assert_eq!(kept.len(), 1);
    assert!(matches!(
        &kept[0],
        AgentAction::CallSkill { skill, .. } if skill == "system_basic"
    ));
}

#[test]
fn scalar_path_observation_does_not_strip_after_tool_output_started() {
    let mut loop_state = LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    let mut route = route_result(
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    route.output_contract.delivery_required = false;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "path_batch_facts",
                "paths": ["/workspace/stem_unique/abcd"],
                "include_missing": true
            }),
        },
        AgentAction::Respond {
            content: "/workspace/stem_unique/abcd".to_string(),
        },
    ];

    let kept = strip_terminal_discussion_for_scalar_path_observation(
        Some(&route),
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
        AgentAction::Respond { content } if content == "/workspace/stem_unique/abcd"
    ));
}

#[test]
fn system_basic_compare_paths_targets_alias_sets_left_and_right_paths() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "compare_paths",
            "targets": ["README.md", "AGENTS.md"],
        }),
    }];

    let normalized = normalize_system_basic_schema_aliases(actions);
    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "system_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("compare_paths")
            );
            assert_eq!(
                args.get("left_path").and_then(|value| value.as_str()),
                Some("README.md")
            );
            assert_eq!(
                args.get("right_path").and_then(|value| value.as_str()),
                Some("AGENTS.md")
            );
        }
        other => panic!("expected system_basic compare_paths action, got {other:?}"),
    }
}

#[test]
fn system_basic_compare_paths_numbered_alias_sets_left_and_right_paths() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "compare_paths",
            "path1": "Cargo.lock",
            "path2": "Cargo.toml",
        }),
    }];

    let normalized = normalize_system_basic_schema_aliases(actions);
    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "system_basic");
            assert_eq!(
                args.get("left_path").and_then(|value| value.as_str()),
                Some("Cargo.lock")
            );
            assert_eq!(
                args.get("right_path").and_then(|value| value.as_str()),
                Some("Cargo.toml")
            );
            assert!(args.get("path1").is_none());
            assert!(args.get("path2").is_none());
        }
        other => panic!("expected system_basic compare_paths action, got {other:?}"),
    }
}

#[test]
fn fs_basic_compare_paths_ab_alias_sets_left_and_right_paths() {
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "compare_paths",
            "path_a": "scripts/a.md",
            "path_b": "scripts/b.md",
        }),
    }];

    let normalized = normalize_fs_basic_schema_aliases(actions);
    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallTool { tool, args } => {
            assert_eq!(tool, "fs_basic");
            assert_eq!(
                args.get("left_path").and_then(Value::as_str),
                Some("scripts/a.md")
            );
            assert_eq!(
                args.get("right_path").and_then(Value::as_str),
                Some("scripts/b.md")
            );
            assert!(args.get("path_a").is_none());
            assert!(args.get("path_b").is_none());
        }
        other => panic!("expected fs_basic compare_paths action, got {other:?}"),
    }
}

#[test]
fn system_basic_path_batch_facts_path_alias_becomes_paths_array() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "path_batch_facts",
            "path": "Cargo.toml",
        }),
    }];

    let normalized = normalize_system_basic_schema_aliases(actions);
    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "system_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("path_batch_facts")
            );
            assert_eq!(args.get("paths"), Some(&json!(["Cargo.toml"])));
            assert!(args.get("path").is_none());
        }
        other => panic!("expected system_basic path_batch_facts action, got {other:?}"),
    }
}

#[test]
fn system_basic_path_batch_facts_path_list_alias_becomes_paths() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "path_batch_facts",
            "path_list": ["Cargo.toml", "Cargo.lock"],
        }),
    }];

    let normalized = normalize_system_basic_schema_aliases(actions);
    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "system_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("path_batch_facts")
            );
            assert_eq!(
                args.get("paths"),
                Some(&json!(["Cargo.toml", "Cargo.lock"]))
            );
            assert!(args.get("path_list").is_none());
        }
        other => panic!("expected system_basic path_batch_facts action, got {other:?}"),
    }
}

#[test]
fn directory_read_range_after_inventory_is_stripped() {
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "inventory_dir",
                "path": "/workspace/docs",
                "sort_by": "mtime_desc",
                "max_entries": 2,
                "names_only": false,
            }),
        },
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "read_range",
                "path": "/workspace/docs/",
                "mode": "head",
                "n": 50,
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec![
                "last_output".to_string(),
                "s1".to_string(),
                "s2".to_string(),
            ],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = strip_directory_read_range_after_inventory_dir(actions);
    assert_eq!(normalized.len(), 3);
    assert!(normalized.iter().all(|action| {
        !matches!(
            action,
            AgentAction::CallSkill { skill, args }
                if skill == "system_basic"
                    && args.get("action").and_then(Value::as_str) == Some("read_range")
        )
    }));
    match &normalized[1] {
        AgentAction::SynthesizeAnswer { evidence_refs } => {
            assert_eq!(
                evidence_refs,
                &vec!["last_output".to_string(), "s1".to_string()]
            );
        }
        other => panic!("expected synthesize_answer after inventory, got {other:?}"),
    }
}

#[test]
fn child_file_read_range_after_inventory_is_kept() {
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({"action": "inventory_dir", "path": "/workspace/docs"}),
        },
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "read_range",
                "path": "/workspace/docs/README.md",
                "mode": "head",
                "n": 20,
            }),
        },
    ];

    let normalized = strip_directory_read_range_after_inventory_dir(actions);
    assert_eq!(normalized.len(), 2);
    assert!(matches!(
        &normalized[1],
        AgentAction::CallSkill { skill, args }
            if skill == "system_basic"
                && args.get("action").and_then(Value::as_str) == Some("read_range")
    ));
}

#[test]
fn unresolved_template_reads_after_inventory_are_stripped() {
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "inventory_dir",
                "path": "/workspace/docs",
                "sort_by": "mtime_desc",
                "max_entries": 2,
            }),
        },
        AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: json!({"path": "{{s1.entry0_path}}"}),
        },
        AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: json!({"path": "{{s1.entry1_path}}"}),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["s1".to_string(), "s2".to_string(), "s3".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = strip_unresolved_template_reads_after_inventory_dir(actions);
    assert_eq!(normalized.len(), 3);
    assert!(normalized.iter().all(|action| {
        !matches!(
            action,
            AgentAction::CallSkill { skill, .. } if skill == "read_file"
        )
    }));
    match &normalized[1] {
        AgentAction::SynthesizeAnswer { evidence_refs } => {
            assert_eq!(evidence_refs, &vec!["s1".to_string()]);
        }
        other => panic!("expected synthesize_answer after inventory, got {other:?}"),
    }
}

#[test]
fn unresolved_template_reads_after_fs_search_are_stripped() {
    let actions = vec![
        AgentAction::CallSkill {
            skill: "fs_search".to_string(),
            args: json!({
                "action": "find_name",
                "pattern": "missing.txt",
                "target_kind": "file",
            }),
        },
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "read_range",
                "path": "{{last_output}}",
                "mode": "head",
                "n": 3,
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["step_1".to_string(), "step_2".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = strip_unresolved_template_reads_after_inventory_dir(actions);
    assert_eq!(normalized.len(), 3);
    assert!(normalized.iter().all(|action| {
        !matches!(
            action,
            AgentAction::CallSkill { skill, args }
                if skill == "system_basic"
                    && args.get("action").and_then(Value::as_str) == Some("read_range")
        )
    }));
    match &normalized[1] {
        AgentAction::SynthesizeAnswer { evidence_refs } => {
            assert_eq!(evidence_refs, &vec!["step_1".to_string()]);
        }
        other => panic!("expected synthesize_answer after fs_search, got {other:?}"),
    }
}

#[test]
fn indexed_last_output_reads_after_inventory_are_kept() {
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "inventory_dir",
                "path": "/workspace/logs",
                "sort_by": "mtime_desc",
                "max_entries": 2,
                "names_only": true,
            }),
        },
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "read_range",
                "path": "/workspace/logs/{{last_output.0}}",
                "mode": "head",
                "n": 40,
            }),
        },
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "read_range",
                "path": "/workspace/logs/{{ last_output[1] }}",
                "mode": "head",
                "n": 40,
            }),
        },
    ];

    let normalized = strip_unresolved_template_reads_after_inventory_dir(actions);
    assert_eq!(normalized.len(), 3);
    assert!(matches!(
        &normalized[1],
        AgentAction::CallSkill { skill, args }
            if skill == "system_basic"
                && args.get("action").and_then(Value::as_str) == Some("read_range")
    ));
    assert!(matches!(
        &normalized[2],
        AgentAction::CallSkill { skill, args }
            if skill == "system_basic"
                && args.get("action").and_then(Value::as_str) == Some("read_range")
    ));
}

#[test]
fn scalar_path_auto_locator_file_builds_observation_plan() {
    let root = TempDirGuard::new("scalar_auto_locator");
    let report = root.path.join("Report.MD");
    fs::write(&report, "hello").expect("write report");
    let report_path = report.display().to_string();
    let route = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "只输出匹配文件路径".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ScalarPathOnly,
            locator_hint: "report.md".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };

    let actions =
        scalar_path_auto_locator_observation_plan(Some(&route), Some(&report_path)).unwrap();
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            assert_eq!(skill, "fs_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("stat_paths")
            );
            assert_eq!(args.get("paths"), Some(&json!([report_path])));
        }
        other => panic!("expected fs_basic stat_paths action, got {other:?}"),
    }
}

#[test]
fn scalar_path_locator_hint_file_builds_observation_plan_before_auto_locator() {
    let root = TempDirGuard::new("scalar_locator_hint");
    let selected = root.path.join("selected.md");
    let other = root.path.join("other.md");
    fs::write(&selected, "selected").expect("write selected");
    fs::write(&other, "other").expect("write other");
    let selected_path = selected.display().to_string();
    let other_path = other.display().to_string();
    let mut route = route_result(
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = selected_path.clone();

    let actions =
        scalar_path_auto_locator_observation_plan(Some(&route), Some(&other_path)).unwrap();

    let args = expect_planned_call(&actions[0], "fs_basic", "stat_paths");
    assert_eq!(args.get("paths"), Some(&json!([selected_path])));
}

#[test]
fn scalar_path_auto_locator_requires_scalar_path_contract() {
    let root = TempDirGuard::new("scalar_auto_locator_requires_contract");
    let selected = root.path.join("selected.md");
    fs::write(&selected, "selected").expect("write selected");
    let selected_path = selected.display().to_string();
    let mut route = route_result(
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = selected_path;

    assert!(scalar_path_auto_locator_observation_plan(Some(&route), None).is_none());
}

#[test]
fn scalar_path_auto_locator_preserves_planner_structural_locator_action() {
    let root = TempDirGuard::new("scalar_auto_locator_deterministic_plan");
    let report = root.path.join("my_abcd.txt");
    fs::write(&report, "hello").expect("write report");
    let report_path = report.display().to_string();
    let mut route = route_result(
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "my_abcd.txt".to_string();
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let normalized = normalize_planned_actions(
        &test_state(),
        Some(&route),
        &loop_state,
        "return the structurally resolved path",
        Some(&report_path),
        vec![AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "stat_paths",
                "paths": [report_path],
                "include_missing": true,
            }),
        }],
    );

    assert_eq!(normalized.len(), 1);
    let args = expect_planned_call(&normalized[0], "fs_basic", "stat_paths");
    assert_eq!(args.get("paths"), Some(&json!([report_path])));
}

#[test]
fn file_basename_auto_locator_preserves_planner_stat_paths_action() {
    let root = TempDirGuard::new("file_basename_auto_locator_deterministic_plan");
    let report = root.path.join("release_checklist.md");
    fs::write(&report, "hello").expect("write report");
    let report_path = report.display().to_string();
    let mut route = route_result(
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::FileBasename;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = report_path.clone();
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let normalized = normalize_planned_actions(
        &test_state(),
        Some(&route),
        &loop_state,
        "return only the selected file basename",
        Some(&report_path),
        vec![AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "stat_paths",
                "paths": [report_path],
                "include_missing": true,
            }),
        }],
    );

    assert_eq!(normalized.len(), 1);
    let args = expect_planned_call(&normalized[0], "fs_basic", "stat_paths");
    assert_eq!(args.get("paths"), Some(&json!([report_path])));
}

#[test]
fn scalar_path_current_workspace_preserves_planner_workspace_contract_action() {
    let root = TempDirGuard::new("scalar_current_workspace_deterministic_plan");
    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let workspace_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::direct_answer(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &loop_state,
        "return current workspace path",
        Some(&workspace_path),
        vec![AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "stat_paths",
                "paths": [workspace_path],
                "include_missing": true,
            }),
        }],
    );

    assert_eq!(normalized.len(), 1);
    let args = expect_planned_call(&normalized[0], "fs_basic", "stat_paths");
    assert_eq!(args.get("paths"), Some(&json!([workspace_path])));
}

#[tokio::test]
async fn plan_round_scalar_path_current_workspace_reaches_planner_without_pre_llm_shortcut() {
    let root = TempDirGuard::new("scalar_current_workspace_plan_round");
    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    state.policy.command_intent.standalone_commands = vec!["pwd".to_string()];
    let prompt = "???? 帮我看看 pwd 是哪儿 :) thx!!!";
    let task = ClaimedTask {
        task_id: "scalar-current-workspace-plan-round".to_string(),
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
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.resolved_intent = "return current workspace path".to_string();
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
        None,
        None,
        Some(&route),
        None,
    )
    .await
    .expect_err("scalar path should now reach planner instead of pre-LLM deterministic plan");

    assert!(
        err.contains("required prompt missing"),
        "expected missing planner prompt after deterministic shortcut removal, got: {err}"
    );
    assert!(
        !err.contains("plan_deterministic_scalar_path_current_workspace"),
        "old scalar path deterministic fallback leaked into planner error: {err}"
    );
}

#[tokio::test]
async fn explicit_command_scalar_path_current_workspace_reaches_planner_path() {
    let root = TempDirGuard::new("explicit_command_scalar_current_workspace_plan_round");
    let mut state = test_state_with_enabled_skills(&["run_cmd", "fs_basic"]);
    state.skill_rt.workspace_root = root.path.clone();
    state.policy.command_intent.execute_prefixes = vec!["执行".to_string()];
    state.policy.command_intent.standalone_commands = vec!["pwd".to_string()];
    let prompt = "执行 pwd，只输出当前工作目录的绝对路径";
    let task = ClaimedTask {
        task_id: "explicit-command-scalar-current-workspace-plan-round".to_string(),
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
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.route_reason = "explicit_command_preserves_structured_observation_contract".to_string();
    route.resolved_intent =
        "Execute pwd command and output only the absolute path of the current working directory."
            .to_string();
    let loop_state = LoopState::new(1);
    let policy = super::super::super::support::load_agent_loop_guard_policy(&state);
    assert!(super::super::explicit_command_request_present(
        &state.policy.command_intent,
        prompt,
        Some(&route)
    ));
    assert!(
        super::super::explicit_command_scalar_path_current_workspace_should_prefer_run_cmd(
            &state.policy.command_intent,
            prompt,
            Some(&route)
        )
    );

    let err = super::super::plan_round_actions(
        &state,
        &task,
        &route.resolved_intent,
        prompt,
        &policy,
        &loop_state,
        None,
        None,
        Some(&route),
        None,
    )
    .await
    .expect_err("explicit current workspace command should reach planner path");

    assert!(
        err.contains("required prompt missing"),
        "expected missing planner prompt after explicit-command preplan removal, got: {err}"
    );
    assert!(
        !err.contains("plan_deterministic_explicit_command_run_cmd"),
        "old explicit-command deterministic fallback leaked into planner error: {err}"
    );
}

#[tokio::test]
async fn explicit_command_scalar_path_auto_locator_conflict_reaches_planner_path() {
    let root = TempDirGuard::new("explicit_command_scalar_auto_locator_conflict");
    let mut state = test_state_with_enabled_skills(&["run_cmd", "fs_basic"]);
    state.skill_rt.workspace_root = root.path.clone();
    state.policy.command_intent.execute_prefixes = vec!["run ".to_string()];
    state.policy.command_intent.standalone_commands = vec!["pwd".to_string()];
    let prompt = "Run pwd and output only the raw result.";
    let task = ClaimedTask {
        task_id: "explicit-command-scalar-auto-locator-conflict-plan-round".to_string(),
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
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root.path.join("run").display().to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.route_reason = concat!(
        "User requests execution of pwd command; ",
        "explicit_command_preserves_structured_observation_contract"
    )
    .to_string();
    route.resolved_intent =
        "Run pwd command and output the raw current working directory.".to_string();
    let loop_state = LoopState::new(1);
    let policy = super::super::super::support::load_agent_loop_guard_policy(&state);
    assert!(super::super::explicit_command_request_present(
        &state.policy.command_intent,
        prompt,
        Some(&route)
    ));
    assert!(
        super::super::explicit_command_scalar_path_current_workspace_should_prefer_run_cmd(
            &state.policy.command_intent,
            prompt,
            Some(&route)
        )
    );

    let err = super::super::plan_round_actions(
        &state,
        &task,
        &route.resolved_intent,
        prompt,
        &policy,
        &loop_state,
        None,
        None,
        Some(&route),
        Some(&route.output_contract.locator_hint),
    )
    .await
    .expect_err("explicit command should reach planner despite auto-locator conflict");

    assert!(
        err.contains("required prompt missing"),
        "expected missing planner prompt after explicit-command preplan removal, got: {err}"
    );
    assert!(
        !err.contains("plan_deterministic_explicit_command_run_cmd"),
        "old explicit-command deterministic fallback leaked into planner error: {err}"
    );
}

#[test]
fn file_facts_auto_locator_builds_stat_paths_synthesis_plan() {
    let root = TempDirGuard::new("file_facts_auto_locator");
    let report = root.path.join("README.md");
    fs::write(&report, "hello").expect("write report");
    let report_path = report.display().to_string();
    let mut route = route_result(
        crate::AskMode::direct_answer(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = report_path.clone();

    let actions =
        file_facts_auto_locator_observation_plan(Some(&route), Some(&report_path)).unwrap();

    assert_eq!(actions.len(), 3);
    match &actions[0] {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            assert_eq!(skill, "fs_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("stat_paths")
            );
            assert_eq!(args.get("paths"), Some(&json!([report_path])));
            assert_eq!(
                args.get("fields"),
                Some(&json!(["exists", "kind", "size", "modified"]))
            );
        }
        other => panic!("expected fs_basic stat_paths action, got {other:?}"),
    }
    assert!(matches!(actions[1], AgentAction::SynthesizeAnswer { .. }));
    assert!(matches!(
        &actions[2],
        AgentAction::Respond { content } if content == "{{last_output}}"
    ));
}

#[test]
fn file_facts_auto_locator_does_not_override_content_semantic() {
    let root = TempDirGuard::new("file_facts_content_semantic");
    let report = root.path.join("README.md");
    fs::write(&report, "hello").expect("write report");
    let report_path = report.display().to_string();
    let mut route = route_result(
        crate::AskMode::act_with_chat_finalizer(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = report_path.clone();

    assert!(file_facts_auto_locator_observation_plan(Some(&route), Some(&report_path)).is_none());
}

#[test]
fn file_facts_auto_locator_accepts_single_file_metadata_mislabeled_as_quantity_comparison() {
    let root = TempDirGuard::new("file_facts_quantity_comparison");
    let report = root.path.join("README.md");
    fs::write(&report, "hello").expect("write report");
    let report_path = report.display().to_string();
    let mut route = route_result(
        crate::AskMode::direct_answer(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = report_path.clone();

    let actions =
        file_facts_auto_locator_observation_plan(Some(&route), Some(&report_path)).unwrap();

    assert!(matches!(
        &actions[0],
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "fs_basic"
                && args.get("action").and_then(Value::as_str) == Some("stat_paths")
                && args.get("paths") == Some(&json!([report_path]))
    ));
    assert!(!should_force_actionable_plan_repair(
        &test_state(),
        Some(&route),
        &LoopState::new(1),
        &actions
    ));
}

#[test]
fn file_facts_auto_locator_uses_route_locator_hint_without_auto_locator_path() {
    let root = TempDirGuard::new("file_facts_quantity_locator_hint");
    let report = root.path.join("README.md");
    fs::write(&report, "hello").expect("write report");
    let report_path = report.display().to_string();
    let mut route = route_result(
        crate::AskMode::direct_answer(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = report_path.clone();

    let actions = file_facts_auto_locator_observation_plan(Some(&route), None).unwrap();

    assert!(matches!(
        &actions[0],
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "fs_basic"
                && args.get("action").and_then(Value::as_str) == Some("stat_paths")
                && args.get("paths") == Some(&json!([report_path]))
    ));
}

#[test]
fn file_facts_auto_locator_accepts_single_directory_metadata_quantity_comparison() {
    let root = TempDirGuard::new("directory_facts_quantity_comparison");
    let target = root.path.join("target");
    fs::create_dir_all(&target).expect("create target dir");
    let target_path = target.display().to_string();
    let mut route = route_result(
        crate::AskMode::direct_answer(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = target_path.clone();

    let actions =
        file_facts_auto_locator_observation_plan(Some(&route), Some(&target_path)).unwrap();

    assert_eq!(actions.len(), 4);
    assert!(matches!(
        &actions[0],
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "fs_basic"
                && args.get("action").and_then(Value::as_str) == Some("stat_paths")
                && args.get("paths") == Some(&json!([target_path.clone()]))
    ));
    assert!(matches!(
        &actions[1],
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "fs_basic"
                && args.get("action").and_then(Value::as_str) == Some("count_entries")
                && args.get("path").and_then(Value::as_str) == Some(target_path.as_str())
                && args.get("recursive").and_then(Value::as_bool) == Some(false)
    ));
    assert!(!should_force_actionable_plan_repair(
        &test_state(),
        Some(&route),
        &LoopState::new(1),
        &actions
    ));
}

#[test]
fn strict_quantity_directory_target_uses_ranked_size_inventory() {
    let root = TempDirGuard::new("directory_facts_quantity_top_files");
    let target = root.path.join("logs");
    fs::create_dir_all(&target).expect("create target dir");
    fs::write(target.join("small.log"), "a").expect("write small");
    fs::write(target.join("large.log"), "abcdef").expect("write large");
    let target_path = target.display().to_string();
    let mut route = route_result(
        crate::AskMode::act_with_chat_finalizer(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = target_path.clone();
    route.output_contract.self_extension.list_selector = crate::OutputListSelector {
        target_kind: crate::OutputScalarCountTargetKind::File,
        target_kind_specified: true,
        limit: Some(3),
        sort_by: Some("size_desc".to_string()),
        include_metadata: Some(true),
        include_hidden: None,
    };
    route.resolved_intent =
        "legacy first-layer summary selector_limit=9 selector_sort_by=mtime_desc".to_string();

    let actions =
        file_facts_auto_locator_observation_plan(Some(&route), Some(&target_path)).unwrap();

    assert_eq!(actions.len(), 3);
    assert!(matches!(
        &actions[0],
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "fs_basic"
                && args.get("action").and_then(Value::as_str) == Some("list_dir")
                && args.get("path").and_then(Value::as_str) == Some(target_path.as_str())
                && args.get("files_only").and_then(Value::as_bool) == Some(true)
                && args.get("sort_by").and_then(Value::as_str) == Some("size_desc")
                && args.get("max_entries").and_then(Value::as_u64) == Some(3)
    ));
}

#[test]
fn strict_quantity_directory_without_selector_uses_path_metadata_plan() {
    let root = TempDirGuard::new("directory_facts_quantity_metadata");
    let target = root.path.join("target");
    fs::create_dir_all(&target).expect("create target dir");
    let target_path = target.display().to_string();
    let mut route = route_result(
        crate::AskMode::act_with_chat_finalizer(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = target_path.clone();
    route.resolved_intent =
        "legacy first-layer summary selector_limit=3 selector_sort_by=size_desc".to_string();

    let actions =
        file_facts_auto_locator_observation_plan(Some(&route), Some(&target_path)).unwrap();

    assert_eq!(actions.len(), 4);
    assert!(matches!(
        &actions[0],
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "fs_basic"
                && args.get("action").and_then(Value::as_str) == Some("stat_paths")
                && args.get("paths") == Some(&json!([target_path.clone()]))
    ));
    assert!(matches!(
        &actions[1],
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "fs_basic"
                && args.get("action").and_then(Value::as_str) == Some("count_entries")
                && args.get("path").and_then(Value::as_str) == Some(target_path.as_str())
    ));
    assert!(!should_force_actionable_plan_repair(
        &test_state(),
        Some(&route),
        &LoopState::new(1),
        &actions
    ));
}

#[test]
fn free_quantity_directory_target_uses_broader_ranked_inventory() {
    let root = TempDirGuard::new("directory_facts_quantity_free_inventory");
    let target = root.path.join("schemas");
    fs::create_dir_all(&target).expect("create target dir");
    fs::write(target.join("small.json"), "{}").expect("write small");
    fs::write(target.join("large.json"), "{\"title\":\"larger\"}").expect("write large");
    let target_path = target.display().to_string();
    let mut route = route_result(
        crate::AskMode::act_with_chat_finalizer(),
        true,
        OutputResponseShape::Free,
    );
    route.resolved_intent =
        "List the selected directory's JSON files and describe the largest one.".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = target_path.clone();

    let actions =
        file_facts_auto_locator_observation_plan(Some(&route), Some(&target_path)).unwrap();

    assert_eq!(actions.len(), 3);
    assert!(matches!(
        &actions[0],
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "fs_basic"
                && args.get("action").and_then(Value::as_str) == Some("list_dir")
                && args.get("path").and_then(Value::as_str) == Some(target_path.as_str())
                && args.get("files_only").and_then(Value::as_bool) == Some(true)
                && args.get("sort_by").and_then(Value::as_str) == Some("size_desc")
                && args.get("max_entries").and_then(Value::as_u64) == Some(50)
    ));
}

#[test]
fn file_facts_auto_locator_preserves_planner_current_workspace_quantity_target() {
    let root = TempDirGuard::new("directory_facts_quantity_current_workspace");
    let target = root.path.join("target");
    fs::create_dir_all(&target).expect("create target dir");
    let target_path = target.canonicalize().expect("canonical target");
    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    state.skill_rt.default_locator_search_dir = root.path.clone();
    let mut route = route_result(
        crate::AskMode::act_with_chat_finalizer(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = "target".to_string();

    let target_path = target_path.display().to_string();
    let loop_state = LoopState::new(1);
    let actions = file_facts_auto_locator_observation_plan(Some(&route), Some(&target_path))
        .expect("planner-observation actions");
    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &loop_state,
        "inspect target metadata",
        Some(&target_path),
        actions,
    );

    assert!(normalized.iter().any(|action| matches!(
        action,
        AgentAction::CallTool { tool, args }
            if tool == "fs_basic"
                && args.get("action").and_then(Value::as_str) == Some("count_entries")
                && args.get("path").and_then(Value::as_str) == Some(target_path.as_str())
    )));
}

#[test]
fn quantity_compare_pair_locator_preserves_planner_compare_paths_action() {
    let root = TempDirGuard::new("quantity_compare_pair_locator");
    fs::write(root.path.join("Cargo.lock"), "abcdef").expect("write lock");
    fs::write(root.path.join("Cargo.toml"), "abc").expect("write toml");
    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    state.skill_rt.default_locator_search_dir = root.path.clone();
    let mut route = route_result(
        crate::AskMode::direct_answer(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "Cargo.lock | Cargo.toml".to_string();

    let loop_state = LoopState::new(1);
    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &loop_state,
        "compare two path metadata targets",
        None,
        vec![AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "compare_paths",
                "left_path": root.path.join("Cargo.lock").display().to_string(),
                "right_path": root.path.join("Cargo.toml").display().to_string(),
            }),
        }],
    );

    let args = normalized
        .iter()
        .filter(|action| planned_call_is(action, "fs_basic", "compare_paths"))
        .map(|action| expect_planned_call(action, "fs_basic", "compare_paths"))
        .next()
        .expect("planner compare_paths action");
    assert!(args
        .get("left_path")
        .and_then(Value::as_str)
        .is_some_and(|path| path.ends_with("Cargo.lock")));
    assert!(args
        .get("right_path")
        .and_then(Value::as_str)
        .is_some_and(|path| path.ends_with("Cargo.toml")));
}

#[test]
fn quantity_compare_pair_locator_preserves_planner_count_entries_for_directory_pairs() {
    let root = TempDirGuard::new("quantity_compare_directory_pair_locator");
    fs::create_dir_all(root.path.join("crates/skills")).expect("write dirs");
    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    state.skill_rt.default_locator_search_dir = root.path.clone();
    let mut route = route_result(
        crate::AskMode::act_with_chat_finalizer(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "crates | crates/skills".to_string();

    let loop_state = LoopState::new(1);
    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &loop_state,
        "count entries in two directories",
        None,
        vec![
            AgentAction::CallTool {
                tool: "fs_basic".to_string(),
                args: json!({
                    "action": "count_entries",
                    "path": root.path.join("crates").display().to_string(),
                    "recursive": false,
                    "include_hidden": false,
                }),
            },
            AgentAction::CallTool {
                tool: "fs_basic".to_string(),
                args: json!({
                    "action": "count_entries",
                    "path": root.path.join("crates/skills").display().to_string(),
                    "recursive": false,
                    "include_hidden": false,
                }),
            },
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec!["step_1".to_string(), "step_2".to_string()],
            },
            AgentAction::Respond {
                content: "{{last_output}}".to_string(),
            },
        ],
    );

    assert!(normalized.len() >= 2);
    let first_args = expect_planned_call(&normalized[0], "fs_basic", "count_entries");
    assert!(first_args
        .get("path")
        .and_then(Value::as_str)
        .is_some_and(|path| path.ends_with("crates")));
    assert_eq!(
        first_args.get("recursive").and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(
        first_args.get("include_hidden").and_then(Value::as_bool),
        Some(false)
    );

    let second_args = expect_planned_call(&normalized[1], "fs_basic", "count_entries");
    assert!(second_args
        .get("path")
        .and_then(Value::as_str)
        .is_some_and(|path| path.ends_with("crates/skills")));
    assert_eq!(
        second_args.get("recursive").and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(
        second_args.get("include_hidden").and_then(Value::as_bool),
        Some(false)
    );
}

#[test]
fn quantity_compare_pair_locator_recovers_pair_from_original_request_over_parent_hint() {
    let root = TempDirGuard::new("quantity_compare_original_request_pair");
    fs::create_dir_all(
        root.path
            .join("scripts/nl_tests/fixtures/device_local/docs"),
    )
    .expect("docs directory");
    fs::create_dir_all(
        root.path
            .join("scripts/nl_tests/fixtures/device_local/logs"),
    )
    .expect("logs directory");
    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    state.skill_rt.default_locator_search_dir = root.path.clone();
    let mut route = route_result(
        crate::AskMode::act_with_chat_finalizer(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "scripts/nl_tests/fixtures/device_local".to_string();
    let prompt = "先数 scripts/nl_tests/fixtures/device_local/docs 直接子项数量，再数 scripts/nl_tests/fixtures/device_local/logs 直接子项数量，最后一句中文说哪个更多";

    let docs_path = root
        .path
        .join("scripts/nl_tests/fixtures/device_local/docs")
        .display()
        .to_string();
    let logs_path = root
        .path
        .join("scripts/nl_tests/fixtures/device_local/logs")
        .display()
        .to_string();
    let loop_state = LoopState::new(1);
    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &loop_state,
        prompt,
        None,
        vec![
            AgentAction::CallTool {
                tool: "fs_basic".to_string(),
                args: json!({
                    "action": "count_entries",
                    "path": docs_path,
                    "recursive": false,
                    "include_hidden": false,
                }),
            },
            AgentAction::CallTool {
                tool: "fs_basic".to_string(),
                args: json!({
                    "action": "count_entries",
                    "path": logs_path,
                    "recursive": false,
                    "include_hidden": false,
                }),
            },
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec!["step_1".to_string(), "step_2".to_string()],
            },
            AgentAction::Respond {
                content: "{{last_output}}".to_string(),
            },
        ],
    );

    assert!(normalized.len() >= 2);
    let first_args = expect_planned_call(&normalized[0], "fs_basic", "count_entries");
    assert!(first_args
        .get("path")
        .and_then(Value::as_str)
        .is_some_and(|path| path.ends_with("/docs")));
    assert_eq!(
        first_args.get("recursive").and_then(Value::as_bool),
        Some(false)
    );

    let second_args = expect_planned_call(&normalized[1], "fs_basic", "count_entries");
    assert!(second_args
        .get("path")
        .and_then(Value::as_str)
        .is_some_and(|path| path.ends_with("/logs")));
    assert_eq!(
        second_args.get("recursive").and_then(Value::as_bool),
        Some(false)
    );
}

#[test]
fn quantity_directory_inventory_injects_structural_extension_filter() {
    let mut route = route_result(
        crate::AskMode::act_with_chat_finalizer(),
        true,
        OutputResponseShape::Strict,
    );
    route.resolved_intent =
        "[CONTRACT_TEST_HINT]\nselector_extension=json\n[/CONTRACT_TEST_HINT]".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "prompts/schemas".to_string();
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "list_dir",
            "path": "prompts/schemas",
            "files_only": true,
            "names_only": false,
            "sort_by": "size_desc",
            "max_entries": 5,
        }),
    }];

    let rewritten =
        inject_structural_extension_filter_for_directory_inventory(Some(&route), "", None, actions);

    let args = expect_planned_call(&rewritten[0], "fs_basic", "list_dir");
    assert_eq!(args.get("ext_filter"), Some(&json!(["json"])));
    assert_eq!(args.get("files_only").and_then(Value::as_bool), Some(true));
    assert_eq!(args.get("dirs_only").and_then(Value::as_bool), Some(false));
    assert_eq!(args.get("max_entries").and_then(Value::as_u64), Some(1000));
}

#[test]
fn directory_entry_groups_inventory_injects_extension_from_machine_token() {
    let mut route = route_result(
        crate::AskMode::act_with_chat_finalizer(),
        true,
        OutputResponseShape::Strict,
    );
    route.resolved_intent =
        "[CONTRACT_TEST_HINT]\nselector_extension=toml\n[/CONTRACT_TEST_HINT]".to_string();
    route.route_reason.clear();
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryEntryGroups;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "list_dir",
            "path": "/workspace",
            "names_only": false,
        }),
    }];

    let rewritten =
        inject_structural_extension_filter_for_directory_inventory(Some(&route), "", None, actions);

    let args = expect_planned_call(&rewritten[0], "fs_basic", "list_dir");
    assert_eq!(args.get("ext_filter"), Some(&json!(["toml"])));
    assert_eq!(args.get("files_only").and_then(Value::as_bool), Some(true));
    assert_eq!(args.get("dirs_only").and_then(Value::as_bool), Some(false));
}

#[test]
fn directory_entry_groups_inventory_ignores_non_extension_user_words() {
    let mut route = route_result(
        crate::AskMode::act_with_chat_finalizer(),
        true,
        OutputResponseShape::Strict,
    );
    route.resolved_intent.clear();
    route.route_reason.clear();
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryEntryGroups;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "list_dir",
            "path": "/workspace",
            "names_only": false,
        }),
    }];

    let rewritten = inject_structural_extension_filter_for_directory_inventory(
        Some(&route),
        "list top-level directory entries",
        None,
        actions,
    );

    let args = expect_planned_call(&rewritten[0], "fs_basic", "list_dir");
    assert!(args.get("ext_filter").is_none());
    assert!(args.get("files_only").is_none());
    assert!(args.get("dirs_only").is_none());
}

#[test]
fn single_path_metadata_facts_do_not_satisfy_multi_target_quantity_comparison() {
    let mut route = route_result(
        crate::AskMode::act_with_chat_finalizer(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "README.md | AGENTS.md".to_string();
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "stat_paths",
                "paths": ["README.md"],
                "fields": ["kind", "size_bytes"]
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    assert!(should_force_actionable_plan_repair(
        &test_state(),
        Some(&route),
        &LoopState::new(1),
        &actions
    ));
}

#[test]
fn explicit_command_planner_action_preserves_pipeline_literal() {
    let mut state = test_state_with_enabled_skills(&["run_cmd"]);
    state.policy.command_intent.execute_prefixes = vec!["执行命令".to_string()];
    let mut route = route_result(
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route.output_contract.requires_content_evidence = true;
    let loop_state = LoopState::new(1);
    let request = "运行命令 `printf rustclaw | wc -c`，只输出数字";

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &loop_state,
        request,
        None,
        vec![AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({
                "command": "printf rustclaw | wc -c",
                "request_text": request,
                "cwd": state.skill_rt.workspace_root.display().to_string(),
                CLAWD_LITERAL_COMMAND_ARG: true,
            }),
        }],
    );

    assert_eq!(normalized.len(), 1);
    let (tool, args) = planned_call(&normalized[0]).expect("run_cmd call");
    assert_eq!(tool, "run_cmd");
    assert_eq!(
        args.get("command").and_then(Value::as_str),
        Some("printf rustclaw | wc -c")
    );
    assert_eq!(args.get(CLAWD_LITERAL_COMMAND_ARG), Some(&json!(true)));
}

#[test]
fn execution_failed_step_code_span_sequence_preserves_planner_multi_run_cmd_actions() {
    let state = test_state_with_enabled_skills(&["run_cmd"]);
    let mut route = route_result(
        crate::AskMode::act_with_chat_finalizer(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ExecutionFailedStep;
    route.output_contract.requires_content_evidence = true;
    let loop_state = LoopState::new(1);
    let request = "请依次执行命令 `echo RC_RENDER_ZH_OK` 和命令 `definitely_missing_command_rustclaw_render_zh_0605`，只告诉我哪一步失败";

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &loop_state,
        request,
        None,
        vec![
            AgentAction::CallSkill {
                skill: "run_cmd".to_string(),
                args: json!({
                    "command": "echo RC_RENDER_ZH_OK",
                    "request_text": request,
                    "cwd": state.skill_rt.workspace_root.display().to_string(),
                    CLAWD_LITERAL_COMMAND_ARG: true,
                    CLAWD_CONTINUE_ON_ERROR_ARG: true,
                }),
            },
            AgentAction::CallSkill {
                skill: "run_cmd".to_string(),
                args: json!({
                    "command": "definitely_missing_command_rustclaw_render_zh_0605",
                    "request_text": request,
                    "cwd": state.skill_rt.workspace_root.display().to_string(),
                    CLAWD_LITERAL_COMMAND_ARG: true,
                    CLAWD_CONTINUE_ON_ERROR_ARG: true,
                }),
            },
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec!["step_1".to_string(), "step_2".to_string()],
            },
            AgentAction::Respond {
                content: "{{last_output}}".to_string(),
            },
        ],
    );

    assert!(normalized.len() >= 2);
    let (first_tool, first_args) = planned_call(&normalized[0]).expect("first run_cmd call");
    let (second_tool, second_args) = planned_call(&normalized[1]).expect("second run_cmd call");
    assert_eq!(first_tool, "run_cmd");
    assert_eq!(second_tool, "run_cmd");
    assert_eq!(
        first_args.get("command").and_then(Value::as_str),
        Some("echo RC_RENDER_ZH_OK")
    );
    assert_eq!(
        second_args.get("command").and_then(Value::as_str),
        Some("definitely_missing_command_rustclaw_render_zh_0605")
    );
    for args in [first_args, second_args] {
        assert_eq!(args.get(CLAWD_LITERAL_COMMAND_ARG), Some(&json!(true)));
        assert_eq!(args.get(CLAWD_CONTINUE_ON_ERROR_ARG), Some(&json!(true)));
    }
    assert!(normalized.iter().any(|action| matches!(
        action,
        AgentAction::SynthesizeAnswer { evidence_refs }
            if evidence_refs == &vec!["step_1".to_string(), "step_2".to_string()]
    )));
}

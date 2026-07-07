use super::*;

#[test]
fn file_paths_contract_rewrites_legacy_list_dir_with_extension_token_to_find_entries() {
    let actions = vec![AgentAction::CallSkill {
        skill: "list_dir".to_string(),
        args: serde_json::json!({
            "path": "/home/guagua/rustclaw",
            "files_only": true,
            "names_only": true
        }),
    }];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::FilePaths;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.resolved_intent =
        "Find all TOML files in the repository and mention representative ones".to_string();

    let normalized = super::super::normalize_planned_actions_with_original(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        &route.resolved_intent,
        Some("find toml files in this repo and briefly mention a few representative ones"),
        None,
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "find_entries");
    assert_eq!(
        args.get("root").and_then(Value::as_str),
        Some("/home/guagua/rustclaw")
    );
    assert_eq!(args.get("ext").and_then(Value::as_str), Some("toml"));
    assert_eq!(
        args.get("target_kind").and_then(Value::as_str),
        Some("file")
    );
    assert_eq!(args.get("recursive").and_then(Value::as_bool), Some(true));
}

#[test]
fn active_anchor_basename_read_uses_bound_directory() {
    let root = TempDirGuard::new("anchor_read_bound_dir");
    let logs = root.path.join("logs");
    fs::create_dir_all(&logs).expect("create logs");
    let selected = logs.join("clawd-codex-current.log");
    fs::write(&selected, "one\ntwo\n").expect("write selected log");
    let selected_path = selected.display().to_string();
    let plan_context = format!(
        "### ACTIVE_EXECUTION_ANCHOR\nfollowup_bound_target: {}\nfollowup_ordered_entries: 1:act_plan.log | 2:clawd-codex-current.log",
        logs.display()
    );
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "read_text_range",
            "path": "clawd-codex-current.log",
            "mode": "tail",
            "n": 2
        }),
    }];

    let normalized = super::super::normalize_planned_actions_with_original_and_context(
        &test_state(),
        Some(&base_route_result()),
        &LoopState::new(1),
        "read selected active entry",
        Some("read selected active entry"),
        Some(&plan_context),
        None,
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "read_text_range");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(selected_path.as_str())
    );
    assert_eq!(args.get("mode").and_then(Value::as_str), Some("tail"));
    assert_eq!(args.get("n").and_then(Value::as_u64), Some(2));
}

#[test]
fn file_paths_contract_enforces_structured_selector_limit_on_find_entries() {
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "find_entries",
            "root": "/home/guagua/rustclaw",
            "ext": "toml",
            "target_kind": "file",
            "max_results": 20,
            "recursive": true
        }),
    }];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::FilePaths;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.self_extension.list_selector.limit = Some(5);
    route.resolved_intent = "find representative toml files and output only paths".to_string();

    let normalized = super::super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "find representative toml files and output only paths",
        None,
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "find_entries");
    assert_eq!(args.get("max_results").and_then(Value::as_u64), Some(5));
    assert_eq!(args.get("ext").and_then(Value::as_str), Some("toml"));
}

#[test]
fn file_paths_contract_uses_original_user_text_selector_limit_token() {
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "find_entries",
            "root": "/home/guagua/rustclaw",
            "ext": "toml",
            "target_kind": "file",
            "max_results": 30,
            "recursive": true
        }),
    }];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::FilePaths;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.resolved_intent =
        "find representative toml files in this repo and output only the paths".to_string();

    let normalized = super::super::normalize_planned_actions_with_original(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        &route.resolved_intent,
        Some("selector_limit=5"),
        None,
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "find_entries");
    assert_eq!(args.get("max_results").and_then(Value::as_u64), Some(5));
}

#[test]
fn file_paths_contract_preserves_allowed_grep_text_and_prunes_disallowed_steps() {
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": "stat_paths",
                "paths": ["/home/guagua/rustclaw/plan/definitely_missing_20260511.md"]
            }),
        },
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": "find_entries",
                "root": "/home/guagua/rustclaw/plan",
                "pattern": "*.md",
                "target_kind": "file"
            }),
        },
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": "grep_text",
                "query": "execution_intent",
                "root": "/home/guagua/rustclaw/plan",
                "pattern": "*.md"
            }),
        },
        AgentAction::CallSkill {
            skill: "transform".to_string(),
            args: serde_json::json!({
                "action": "transform_data",
                "data": [],
                "ops": []
            }),
        },
    ];
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::FilePaths;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "plan".to_string();
    route.resolved_intent =
        "If the first path is missing, search plan for execution_intent md files and return paths."
            .to_string();

    let normalized = super::super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        &route.resolved_intent,
        None,
        actions,
    );

    assert_eq!(normalized.len(), 2, "normalized actions: {normalized:?}");
    let first = expect_planned_call(&normalized[0], "fs_basic", "find_entries");
    assert_eq!(
        first.get("root").and_then(Value::as_str),
        Some("/home/guagua/rustclaw/plan")
    );
    assert_eq!(first.get("pattern").and_then(Value::as_str), Some("*.md"));
    let second = expect_planned_call(&normalized[1], "fs_basic", "grep_text");
    assert_eq!(
        second.get("root").and_then(Value::as_str),
        Some("/home/guagua/rustclaw/plan")
    );
    assert_eq!(
        second.get("query").and_then(Value::as_str),
        Some("execution_intent")
    );
    assert_eq!(second.get("pattern").and_then(Value::as_str), Some("*.md"));
    assert!(second.get("ext").is_none());
    assert!(second.get("target_kind").is_none());
}

#[test]
fn file_paths_contract_rewrites_fs_basic_list_dir_extension_filter_to_recursive_find() {
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "list_dir",
            "path": "scripts/nl_tests/fixtures/device_local",
            "ext_filter": ".log",
            "names_only": true
        }),
    }];
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::FilePaths;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "scripts/nl_tests/fixtures/device_local".to_string();

    let normalized = super::super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "list matching file paths under a directory",
        None,
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "find_entries");
    assert_eq!(
        args.get("root").and_then(Value::as_str),
        Some("scripts/nl_tests/fixtures/device_local")
    );
    assert_eq!(args.get("ext").and_then(Value::as_str), Some("log"));
    assert_eq!(
        args.get("target_kind").and_then(Value::as_str),
        Some("file")
    );
    assert_eq!(args.get("recursive").and_then(Value::as_bool), Some(true));
}

#[test]
fn file_paths_contract_preserves_planned_synthesis_selection() {
    let actions = vec![
        AgentAction::CallSkill {
            skill: "fs_search".to_string(),
            args: serde_json::json!({
                "action": "find_name",
                "root": ".",
                "name": "*.toml",
                "max_results": 50
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
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::FilePaths;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;

    let normalized = super::super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "return five representative TOML file paths from the repository",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 2);
    assert!(matches!(
        &normalized[0],
        AgentAction::CallSkill { skill, .. } | AgentAction::CallTool { tool: skill, .. }
            if skill == "fs_basic"
    ));
    assert!(matches!(
        &normalized[1],
        AgentAction::SynthesizeAnswer { evidence_refs } if evidence_refs == &vec!["last_output".to_string()]
    ));
}

#[test]
fn file_paths_anchor_respond_only_adds_find_entries_evidence() {
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::FilePaths;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/locator_smart/fuzzy_top3".to_string();
    let selected = "scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/my_abcd.txt";
    let plan_context = "\
### ACTIVE_EXECUTION_ANCHOR
followup_bound_target: /home/guagua/rustclaw/scripts/nl_tests/fixtures/locator_smart/fuzzy_top3
followup_ordered_entries: 1:scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/abcd_report.md | 2:scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/my_abcd.txt | 3:scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/x_abcd_log.txt
";
    let actions = vec![AgentAction::Respond {
        content: selected.to_string(),
    }];

    let normalized = super::super::normalize_planned_actions_with_original_and_context(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "select the second path",
        None,
        Some(plan_context),
        Some("scripts/nl_tests/fixtures/locator_smart/fuzzy_top3"),
        actions,
    );

    assert_eq!(normalized.len(), 1);
    let args = expect_planned_call(&normalized[0], "fs_basic", "find_entries");
    assert_eq!(
        args.get("root").and_then(Value::as_str),
        Some("scripts/nl_tests/fixtures/locator_smart/fuzzy_top3")
    );
    assert_eq!(
        args.get("pattern").and_then(Value::as_str),
        Some("my_abcd.txt")
    );
    assert_eq!(
        args.get("target_kind").and_then(Value::as_str),
        Some("file")
    );
}

#[test]
fn scalar_path_anchor_respond_only_adds_stat_paths_evidence() {
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/locator_smart/fuzzy_top3".to_string();
    let selected = "scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/my_abcd.txt";
    let plan_context = "\
### ACTIVE_EXECUTION_ANCHOR
followup_bound_target: /home/guagua/rustclaw/scripts/nl_tests/fixtures/locator_smart/fuzzy_top3
followup_ordered_entries: 1:scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/abcd_report.md | 2:scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/my_abcd.txt | 3:scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/x_abcd_log.txt
";
    let actions = vec![AgentAction::Respond {
        content: selected.to_string(),
    }];

    let normalized = super::super::normalize_planned_actions_with_original_and_context(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "select the second path",
        None,
        Some(plan_context),
        Some("scripts/nl_tests/fixtures/locator_smart/fuzzy_top3"),
        actions,
    );

    assert_eq!(normalized.len(), 1);
    let args = expect_planned_call(&normalized[0], "fs_basic", "stat_paths");
    assert_eq!(
        args.get("paths")
            .and_then(Value::as_array)
            .and_then(|items| { items.first().and_then(Value::as_str).map(str::to_string) }),
        Some(selected.to_string())
    );
    assert_eq!(
        args.get("include_missing").and_then(Value::as_bool),
        Some(true)
    );
}

#[test]
fn file_paths_contract_normalizes_fs_search_glob_extension_args() {
    let root = TempDirGuard::new("fs_search_file_paths_contract");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::FilePaths;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    let actions = vec![AgentAction::CallSkill {
        skill: "fs_search".to_string(),
        args: json!({
            "action": "find_name",
            "basename_pattern": "*.toml",
            "search_root": root_path,
            "type": "file",
            "max_results": 5
        }),
    }];

    let normalized = super::super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "return five representative TOML file paths from the repository",
        None,
        actions,
    );

    match &normalized[0] {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            assert_eq!(skill, "fs_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("find_entries")
            );
            assert_eq!(
                args.get("root").and_then(Value::as_str),
                Some(root_path.as_str())
            );
            assert_eq!(args.get("ext").and_then(Value::as_str), Some("toml"));
            assert_eq!(args.get("max_results").and_then(Value::as_u64), Some(5));
        }
        other => panic!("expected normalized fs_basic action, got {other:?}"),
    }
}

#[test]
fn observation_only_terminal_answer_keeps_raw_command_runtime_finalizer() {
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: serde_json::json!({ "command": "pwd" }),
    }];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        false,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;

    let normalized = super::super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "执行 pwd，直接输出命令结果",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 1);
    assert!(matches!(
        &normalized[0],
        AgentAction::CallSkill { skill, .. } if skill == "run_cmd"
    ));
}

#[test]
fn raw_command_output_keeps_user_named_new_file_write_path_plan() {
    let root = TempDirGuard::new("raw_command_user_named_output_write");
    let mut state = test_state_with_registry();
    state.skill_rt.workspace_root = root.path.clone();
    let file_path = root.path.join("pwd_line_abs.txt");
    let request = "Run pwd, write one short line based on it into pwd_line_abs.txt, and reply with the absolute saved file path only.";
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_hint.clear();
    let actions = vec![
        AgentAction::CallTool {
            tool: "run_cmd".to_string(),
            args: json!({
                "command": "pwd",
                "cwd": root.path.display().to_string()
            }),
        },
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "write_text",
                "path": file_path.display().to_string(),
                "text": format!("{}\n", root.path.display())
            }),
        },
        AgentAction::Respond {
            content: file_path.display().to_string(),
        },
    ];

    let normalized = super::super::normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &LoopState::new(2),
        request,
        Some(request),
        None,
        actions,
    );

    assert!(normalized
        .iter()
        .any(|action| { planned_call_is(action, "fs_basic", "write_text") }));
    let write_args = normalized
        .iter()
        .find_map(|action| action_capability_and_action(action, "fs_basic", "write_text"))
        .expect("fs_basic write_text should remain");
    assert_eq!(
        write_args
            .get(super::super::super::CLAWD_USER_NAMED_OUTPUT_PATH_ARG)
            .and_then(Value::as_bool),
        Some(true)
    );
    assert!(!normalized
        .iter()
        .any(|action| { planned_call_is(action, "process_basic", "ps") }));
}

#[test]
fn workspace_summary_keeps_requested_structured_field_evidence() {
    let actions = vec![
        AgentAction::CallSkill {
            skill: "list_dir".to_string(),
            args: serde_json::json!({ "path": "." }),
        },
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "extract_field",
                "path": "UI/package.json",
                "field_path": "name"
            }),
        },
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "read_range",
                "path": "README.md",
                "mode": "head",
                "n": 10
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec![
                "step_1".to_string(),
                "step_2".to_string(),
                "step_3".to_string(),
            ],
        },
    ];
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::WorkspaceProjectSummary;
    route.resolved_intent =
        "先看顶层目录，再读 UI/package.json 的 name，最后一句话判断 UI 定位".to_string();

    let normalized = super::super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        &route.resolved_intent,
        None,
        actions,
    );
    assert!(normalized.iter().any(|action| matches!(
        action,
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "config_basic"
                && args.get("action").and_then(|value| value.as_str()) == Some("read_field")
    )));
}

#[test]
fn workspace_summary_with_scope_prunes_sibling_evidence() {
    let actions = vec![
        AgentAction::CallSkill {
            skill: "list_dir".to_string(),
            args: serde_json::json!({ "path": "UI" }),
        },
        AgentAction::CallSkill {
            skill: "list_dir".to_string(),
            args: serde_json::json!({ "path": "pi_app" }),
        },
    ];
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::WorkspaceProjectSummary;
    route.output_contract.locator_hint = "UI".to_string();
    route.resolved_intent = "Summarize only the UI part of this repository".to_string();

    let pruned = super::super::prune_unscoped_workspace_summary_evidence_for_scope(
        &test_state(),
        Some(&route),
        actions,
    );
    assert_eq!(pruned.len(), 1);
    match &pruned[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "list_dir");
            assert_eq!(
                args.get("path").and_then(|value| value.as_str()),
                Some("UI")
            );
        }
        other => panic!("expected scoped UI list_dir action, got {other:?}"),
    }
}

#[test]
fn workspace_root_identity_scope_keeps_relative_workspace_evidence() {
    let root = TempDirGuard::new("rustclaw");
    fs::write(root.path.join("README.md"), "# RustClaw").expect("write README");
    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::WorkspaceProjectSummary;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = root
        .path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap()
        .to_string();

    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({"action": "tree_summary", "path": root.path.display().to_string()}),
        },
        AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: json!({"path": "README.md"}),
        },
    ];

    let pruned = super::super::prune_unscoped_workspace_summary_evidence_for_scope(
        &state,
        Some(&route),
        actions,
    );

    assert_eq!(pruned.len(), 2);
    assert!(matches!(
        &pruned[1],
        AgentAction::CallSkill { skill, args }
            if skill == "read_file"
                && args.get("path").and_then(|value| value.as_str()) == Some("README.md")
    ));
}

#[test]
fn unscoped_workspace_evidence_appends_synthesis_after_existing_text_read_plan() {
    let root = TempDirGuard::new("workspace_text_evidence_existing");
    fs::write(root.path.join("README.md"), "# RustClaw").expect("write README");
    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_hint.clear();
    let actions = vec![AgentAction::CallSkill {
        skill: "read_file".to_string(),
        args: json!({"path":"README.md"}),
    }];

    let normalized = super::super::normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        &route.resolved_intent,
        None,
        actions,
    );
    assert_eq!(normalized.len(), 3);
    assert!(planned_call_is(
        &normalized[0],
        "fs_basic",
        "read_text_range"
    ));
    assert!(matches!(
        &normalized[1],
        AgentAction::SynthesizeAnswer { evidence_refs }
            if evidence_refs == &vec!["step_1".to_string()]
    ));
    assert!(matches!(
        &normalized[2],
        AgentAction::Respond { content } if content == "{{last_output}}"
    ));
}

#[test]
fn unscoped_workspace_text_answer_strips_unrequested_file_artifact_plan() {
    let root = TempDirGuard::new("workspace_text_evidence_no_artifact");
    fs::write(
        root.path.join("README.md"),
        "# RustClaw\n\nUse the documented installer",
    )
    .expect("write README");
    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.delivery_required = false;
    route.wants_file_delivery = false;
    route.resolved_intent = "Write a short RustClaw setup note".to_string();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: json!({"path":"Cargo.toml"}),
        },
        AgentAction::CallSkill {
            skill: "write_file".to_string(),
            args: json!({
                "path":"document/SETUP_NOTE.md",
                "content":"# RustClaw Setup Note\n"
            }),
        },
        AgentAction::Respond {
            content: "FILE:/home/guagua/rustclaw/document/SETUP_NOTE.md".to_string(),
        },
    ];

    let normalized = super::super::normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        &route.resolved_intent,
        None,
        actions,
    );
    assert!(normalized.iter().all(|action| {
        !matches!(
            action,
            AgentAction::CallSkill { skill, .. } if skill == "write_file"
        ) && !planned_call_is(action, "fs_basic", "write_text")
    }));
    assert!(normalized.iter().all(|action| {
        !matches!(
            action,
            AgentAction::Respond { content } if content.trim().starts_with("FILE:")
        )
    }));
    assert!(normalized.iter().any(|action| {
        matches!(
            action,
            AgentAction::SynthesizeAnswer { evidence_refs }
                if evidence_refs == &vec!["step_1".to_string()]
        )
    }));
    assert!(matches!(
        normalized.last(),
        Some(AgentAction::Respond { content }) if content == "{{last_output}}"
    ));
}

#[test]
fn active_execution_recipe_keeps_workspace_file_mutation_plan() {
    let root = TempDirGuard::new("workspace_text_evidence_recipe_mutation");
    fs::write(root.path.join("README.md"), "# RustClaw").expect("write README");
    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_hint.clear();
    let mut loop_state = LoopState::new(1);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
        ..Default::default()
    };
    let actions = vec![AgentAction::CallSkill {
        skill: "write_file".to_string(),
        args: json!({
            "path":"document/SETUP_NOTE.md",
            "content":"# RustClaw Setup Note\n"
        }),
    }];

    let normalized = super::super::normalize_planned_actions(
        &state,
        Some(&route),
        &loop_state,
        &route.resolved_intent,
        None,
        actions,
    );
    assert!(normalized
        .iter()
        .any(|action| { planned_call_is(action, "fs_basic", "write_text") }));
}

#[test]
fn explicit_workspace_file_locator_keeps_requested_file_mutation_plan() {
    let root = TempDirGuard::new("workspace_text_evidence_requested_mutation");
    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_hint = "plan/p2_expand_test.md".to_string();
    route.resolved_intent = "Create plan/p2_expand_test.md and write p2 hello".to_string();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "make_dir".to_string(),
            args: json!({"path":"plan"}),
        },
        AgentAction::CallSkill {
            skill: "write_file".to_string(),
            args: json!({
                "path":"plan/p2_expand_test.md",
                "content":"p2 hello"
            }),
        },
        AgentAction::Respond {
            content: "created".to_string(),
        },
    ];

    let normalized = super::super::normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        &route.resolved_intent,
        None,
        actions,
    );
    assert!(normalized
        .iter()
        .any(|action| { planned_call_is(action, "fs_basic", "make_dir") }));
    assert!(normalized
        .iter()
        .any(|action| { planned_call_is(action, "fs_basic", "write_text") }));
}

#[test]
fn delivery_write_strips_redundant_make_dir_and_appends_file_token() {
    let root = TempDirGuard::new("delivery_write_generic");
    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        false,
        OutputResponseShape::FileToken,
    );
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = OutputDeliveryIntent::FileSingle;
    route.output_contract.response_shape = OutputResponseShape::FileToken;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "document/manual_meta.json".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.wants_file_delivery = true;
    route.resolved_intent =
        "Generate document/manual_meta.json and send the file to the user.".to_string();
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "make_dir",
                "path": "document"
            }),
        },
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "write_text",
                "path": "document/manual_meta.json",
                "content": "{\"app\":\"RustClaw\",\"test\":\"nl\"}"
            }),
        },
    ];

    let normalized = super::super::normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        &route.resolved_intent,
        None,
        actions,
    );

    assert!(!normalized
        .iter()
        .any(|action| { planned_call_is(action, "fs_basic", "make_dir") }));
    assert!(normalized
        .iter()
        .any(|action| { planned_call_is(action, "fs_basic", "write_text") }));
    let expected = format!("FILE:{}/document/manual_meta.json", root.path.display());
    assert!(matches!(
        normalized.last(),
        Some(AgentAction::Respond { content }) if content == &expected
    ));
}

#[test]
fn free_route_strips_terminal_discussion_after_runner_skill() {
    let state = test_state();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "crypto".to_string(),
            args: serde_json::json!({ "action": "quote", "symbol": "BTCUSDT" }),
        },
        AgentAction::Respond {
            content: "下面是我帮你整理后的结果。".to_string(),
        },
    ];

    let stripped = strip_terminal_discussion_for_direct_skill_passthrough(
        &state,
        Some(&route_result(
            crate::AskMode::planner_execute_plain(),
            false,
            OutputResponseShape::Free,
        )),
        actions,
    );
    assert_eq!(stripped.len(), 1);
    assert!(matches!(
        &stripped[0],
        AgentAction::CallSkill { skill, .. } if skill == "crypto"
    ));
}

#[test]
fn process_basic_port_list_keeps_terminal_discussion_followup() {
    let state = test_state();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "process_basic".to_string(),
            args: serde_json::json!({ "action": "port_list" }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let kept = strip_terminal_discussion_for_direct_skill_passthrough(
        &state,
        Some(&route_result(
            crate::AskMode::planner_execute_with_chat_finalizer(),
            false,
            OutputResponseShape::Free,
        )),
        actions.clone(),
    );
    assert_eq!(kept.len(), 3);
    assert!(matches!(
        &kept[0],
        AgentAction::CallSkill { skill, args }
            if skill == "process_basic"
                && args.get("action").and_then(|value| value.as_str()) == Some("port_list")
    ));
    assert!(matches!(
        &kept[1],
        AgentAction::SynthesizeAnswer { evidence_refs }
            if evidence_refs == &vec!["last_output".to_string()]
    ));
    assert!(matches!(
        &kept[2],
        AgentAction::Respond { content } if content == "{{last_output}}"
    ));
}

#[test]
fn service_status_process_basic_port_list_keeps_terminal_synthesis() {
    let state = test_state();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "process_basic".to_string(),
            args: serde_json::json!({ "action": "port_list" }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let stripped =
        strip_terminal_discussion_for_direct_skill_passthrough(&state, Some(&route), actions);

    assert_eq!(stripped.len(), 3);
    assert!(matches!(
        &stripped[0],
        AgentAction::CallSkill { skill, args }
            if skill == "process_basic"
                && args.get("action").and_then(Value::as_str) == Some("port_list")
    ));
    assert!(matches!(
        &stripped[1],
        AgentAction::SynthesizeAnswer { evidence_refs }
            if evidence_refs == &vec!["last_output".to_string()]
    ));
    assert!(matches!(
        &stripped[2],
        AgentAction::Respond { content } if content == "{{last_output}}"
    ));
}

#[test]
fn service_status_process_basic_port_list_does_not_direct_finalize_model_language_shape() {
    let state = test_state();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    let actions = vec![AgentAction::CallSkill {
        skill: "process_basic".to_string(),
        args: serde_json::json!({ "action": "port_list" }),
    }];

    assert!(!observation_only_plan_can_finalize_from_direct_output(
        &state,
        Some(&route),
        &actions,
    ));
}

#[test]
fn process_basic_synthesis_survives_workspace_text_guard_for_exact_sentence() {
    let state = test_state();
    let loop_state = LoopState::new(1);
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.exact_sentence_count = Some(1);
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "process_basic".to_string(),
            args: serde_json::json!({ "action": "port_list" }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &loop_state,
        &route.resolved_intent,
        None,
        actions,
    );

    assert_eq!(normalized.len(), 3);
    assert!(matches!(
        &normalized[0],
        AgentAction::CallSkill { skill, args }
            if skill == "process_basic"
                && args.get("action").and_then(Value::as_str) == Some("port_list")
    ));
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
fn output_template_code_span_is_not_treated_as_literal_command() {
    let request = "Read Cargo.toml version and answer as `version=<value>` only.";
    assert!(super::super::shellish_literal_command_segment(request).is_none());
}

#[test]
fn colon_output_template_code_span_is_not_treated_as_literal_command() {
    let request = "Return the current git branch in the format `branch: NAME`.";
    assert!(super::super::shellish_literal_command_segment(request).is_none());
}

#[test]
fn concrete_shell_code_span_still_uses_literal_command_path() {
    let request = "Check current directory with `pwd && ls Cargo.toml`.";
    assert_eq!(
        super::super::shellish_literal_command_segment(request).as_deref(),
        Some("pwd && ls Cargo.toml")
    );
}

#[test]
fn direct_passthrough_keeps_mixed_placeholder_terminal_respond() {
    let state = test_state();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: serde_json::json!({ "command": "pwd" }),
        },
        AgentAction::Respond {
            content: "{{last_output}}\n\nworkspace ready".to_string(),
        },
    ];

    let kept =
        strip_terminal_discussion_for_direct_skill_passthrough(&state, Some(&route), actions);
    assert_eq!(kept.len(), 2);
    assert!(matches!(
        &kept[0],
        AgentAction::CallSkill { skill, .. } if skill == "run_cmd"
    ));
    assert!(matches!(
        &kept[1],
        AgentAction::Respond { content } if content.contains("workspace ready")
    ));
}

#[test]
fn strict_run_cmd_template_preserves_mixed_last_output_respond() {
    let state = test_state_with_enabled_skills(&["run_cmd"]);
    let loop_state = LoopState::new(2);
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({ "command": "pwd" }),
        },
        AgentAction::Respond {
            content: "cwd={{last_output}}".to_string(),
        },
    ];

    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &loop_state,
        "运行 pwd 命令，并按 key=value 模板返回当前目录。",
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
                "args": { "command": "pwd" }
            },
            {
                "type": "respond",
                "content": "cwd={{last_output}}"
            }
        ])
    );
}

#[test]
fn runner_skill_only_plan_does_not_require_terminal_respond() {
    let loop_state = LoopState::new(2);
    let actions = vec![AgentAction::CallSkill {
        skill: "crypto".to_string(),
        args: serde_json::json!({ "action": "quote", "symbol": "BTCUSDT" }),
    }];
    assert!(!should_force_plan_repair(
        Some(&route_result(
            crate::AskMode::planner_execute_plain(),
            false,
            OutputResponseShape::Free,
        )),
        &loop_state,
        &actions,
    ));
}

#[test]
fn chat_wrapped_execution_route_repairs_observation_only_plan_before_any_observation() {
    let loop_state = LoopState::new(2);
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: serde_json::json!({ "command": "ls -l Cargo.toml Cargo.lock" }),
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
fn chat_wrapped_execution_route_repairs_observation_plus_unavailable_followup_plan() {
    let state = test_state_with_enabled_skills(&["run_cmd"]);
    let loop_state = LoopState::new(2);
    let actions = vec![
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: serde_json::json!({ "command": "ls -l Cargo.toml Cargo.lock" }),
        },
        AgentAction::CallSkill {
            skill: "formatter".to_string(),
            args: serde_json::json!({ "text": "explain {{last_output}}" }),
        },
    ];
    let route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
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
fn chat_wrapped_execution_route_keeps_observation_plus_synthesize_followup_plan() {
    let loop_state = LoopState::new(2);
    let actions = vec![
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: serde_json::json!({ "command": "ls -l Cargo.toml Cargo.lock" }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
    ];
    assert!(!should_force_plan_repair(
        Some(&route_result(
            crate::AskMode::planner_execute_with_chat_finalizer(),
            false,
            OutputResponseShape::Free,
        )),
        &loop_state,
        &actions,
    ));
}

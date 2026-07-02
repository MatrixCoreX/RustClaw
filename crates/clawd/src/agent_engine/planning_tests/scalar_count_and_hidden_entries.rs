use super::*;

#[test]
fn scalar_count_listing_plan_preserves_dirs_only_dimension_for_count_inventory() {
    let root = TempDirGuard::new("scalar_count_dirs_only_locator_dir");
    fs::write(root.path.join("a.txt"), "a").expect("write a");
    fs::create_dir_all(root.path.join("child")).expect("create child");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_path.clone();
    route.output_contract.delivery_required = false;
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "count_entries",
                "path": root_path.clone(),
                "dirs_only": true,
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = replace_scalar_count_plan_with_count_inventory(
        Some(&route),
        &LoopState::new(1),
        Some(&root_path),
        "count directories",
        actions,
    );

    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallTool { tool: skill, args } => {
            assert_eq!(skill, "fs_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("count_entries")
            );
            assert_eq!(args.get("kind_filter").and_then(Value::as_str), Some("dir"));
            assert_eq!(args.get("count_dirs").and_then(Value::as_bool), Some(true));
            assert_eq!(
                args.get("count_files").and_then(Value::as_bool),
                Some(false)
            );
            assert_eq!(args.get("dirs_only").and_then(Value::as_bool), Some(true));
        }
        other => panic!("expected fs_basic count_entries action, got {other:?}"),
    }
}

#[test]
fn scalar_count_uses_active_listing_target_when_route_is_locatorless() {
    let root = TempDirGuard::new("scalar_count_active_listing_locatorless");
    fs::write(root.path.join("release_checklist.md"), "release").expect("write release");
    fs::write(root.path.join("service_notes.md"), "service").expect("write service");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.route_reason = "active_listing_target_required".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::new(1);
    loop_state.output_vars.insert(
        "active_listing_bound_targets".to_string(),
        json!([root_path.clone()]).to_string(),
    );

    let normalized = replace_scalar_count_plan_with_count_inventory(
        Some(&route),
        &loop_state,
        None,
        "count active listing entries",
        Vec::new(),
    );

    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallTool { tool: skill, args } => {
            assert_eq!(skill, "fs_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("count_entries")
            );
            assert_eq!(
                args.get("path").and_then(Value::as_str),
                Some(root_path.as_str())
            );
        }
        other => panic!("expected fs_basic count_entries action, got {other:?}"),
    }
    assert_eq!(route.output_contract.locator_kind, OutputLocatorKind::None);
    assert!(route.output_contract.locator_hint.is_empty());
}

#[test]
fn scalar_count_uses_current_workspace_scope_target_without_route_prebind() {
    let root = TempDirGuard::new("scalar_count_current_workspace_scope");
    fs::write(root.path.join("release_checklist.md"), "release").expect("write release");
    fs::write(root.path.join("service_notes.md"), "service").expect("write service");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.route_reason = "current_workspace_scope_from_current_request".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::new(1);
    loop_state.output_vars.insert(
        "current_workspace_scalar_count_targets".to_string(),
        json!([root_path.clone()]).to_string(),
    );

    let normalized = replace_scalar_count_plan_with_count_inventory(
        Some(&route),
        &loop_state,
        None,
        "count current workspace entries",
        Vec::new(),
    );

    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallTool { tool: skill, args } => {
            assert_eq!(skill, "fs_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("count_entries")
            );
            assert_eq!(
                args.get("path").and_then(Value::as_str),
                Some(root_path.as_str())
            );
        }
        other => panic!("expected fs_basic count_entries action, got {other:?}"),
    }
    assert_eq!(route.output_contract.locator_kind, OutputLocatorKind::None);
    assert!(route.output_contract.locator_hint.is_empty());
}

#[test]
fn scalar_count_preferred_count_entries_inherits_dirs_filter_from_rejected_list_dir() {
    let root = TempDirGuard::new("scalar_count_rejected_list_dir_dirs_filter");
    fs::create_dir_all(root.path.join(".git")).expect("create git dir");
    fs::create_dir_all(root.path.join("child")).expect("create child dir");
    fs::write(root.path.join("a.txt"), "a").expect("write file");
    let root_path = root.path.display().to_string();
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.path.clone();
    state.skill_rt.default_locator_search_dir = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = root_path.clone();
    route.output_contract.delivery_required = false;
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "list_dir",
                "path": root_path.clone(),
                "dirs_only": true,
                "include_hidden": true,
                "names_only": true,
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &LoopState::new(1),
        "count top-level workspace directories excluding VCS control paths",
        None,
        Some(&root_path),
        actions,
    );

    match &normalized[0] {
        AgentAction::CallTool { tool: skill, args } => {
            assert_eq!(skill, "fs_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("count_entries")
            );
            assert_eq!(
                args.get("path").and_then(Value::as_str),
                Some(root_path.as_str())
            );
            assert_eq!(args.get("kind_filter").and_then(Value::as_str), Some("dir"));
            assert_eq!(args.get("dirs_only").and_then(Value::as_bool), Some(true));
            assert_ne!(args.get("files_only").and_then(Value::as_bool), Some(true));
            assert_eq!(
                args.get("include_hidden").and_then(Value::as_bool),
                Some(false)
            );
        }
        other => panic!("expected fs_basic count_entries action, got {other:?}"),
    }
}

#[test]
fn scalar_count_state_patch_filter_plans_structured_dir_count() {
    let root = TempDirGuard::new("scalar_count_state_patch_filter");
    fs::create_dir_all(root.path.join(".git")).expect("create git dir");
    fs::create_dir_all(root.path.join("child")).expect("create child dir");
    fs::write(root.path.join("a.txt"), "a").expect("write file");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::direct_answer(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = root_path.clone();
    route.output_contract.delivery_required = false;
    let turn_analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::TaskRequest),
        target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
        should_interrupt_active_run: false,
        state_patch: Some(json!({
            "scalar_count_filter": {
                "target_kind": "dir",
                "include_hidden": false,
                "recursive": false
            }
        })),
        attachment_processing_required: false,
    };

    let plan = scalar_count_filter_deterministic_plan_result(
        "count workspace dirs",
        Some(&route),
        &LoopState::new(1),
        Some(&turn_analysis),
        Some(&root_path),
    )
    .expect("structured scalar count plan");

    assert_eq!(plan.steps.len(), 1);
    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "fs_basic", "count_entries");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(root_path.as_str())
    );
    assert_eq!(args.get("kind_filter").and_then(Value::as_str), Some("dir"));
    assert_eq!(args.get("count_dirs").and_then(Value::as_bool), Some(true));
    assert_eq!(
        args.get("count_files").and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(args.get("dirs_only").and_then(Value::as_bool), Some(true));
    assert_eq!(args.get("files_only").and_then(Value::as_bool), Some(false));
    assert_eq!(
        args.get("include_hidden").and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(args.get("recursive").and_then(Value::as_bool), Some(false));
}

#[test]
fn scalar_count_strict_single_sentence_shape_plans_structured_file_count() {
    let root = TempDirGuard::new("scalar_count_strict_single_sentence");
    fs::create_dir_all(root.path.join("child")).expect("create child dir");
    fs::write(root.path.join("a.txt"), "a").expect("write file");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.exact_sentence_count = Some(1);
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = root_path.clone();
    route.output_contract.delivery_required = false;
    route.output_contract.self_extension.scalar_count_filter = crate::OutputScalarCountFilter {
        target_kind: crate::OutputScalarCountTargetKind::File,
        include_hidden: Some(false),
        recursive: Some(false),
        extensions: Vec::new(),
    };
    let plan = scalar_count_filter_deterministic_plan_result(
        "count workspace files and explain briefly",
        Some(&route),
        &LoopState::new(1),
        None,
        Some(&root_path),
    )
    .expect("structured strict scalar count plan");

    assert_eq!(plan.steps.len(), 1);
    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "fs_basic", "count_entries");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(root_path.as_str())
    );
    assert_eq!(
        args.get("kind_filter").and_then(Value::as_str),
        Some("file")
    );
    assert_eq!(args.get("count_files").and_then(Value::as_bool), Some(true));
    assert_eq!(args.get("count_dirs").and_then(Value::as_bool), Some(false));
    assert_eq!(args.get("files_only").and_then(Value::as_bool), Some(true));
    assert_eq!(args.get("dirs_only").and_then(Value::as_bool), Some(false));
    assert_eq!(
        args.get("include_hidden").and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(args.get("recursive").and_then(Value::as_bool), Some(false));
}

#[test]
fn scalar_count_contract_filter_repairs_existing_count_entries_action() {
    let root = TempDirGuard::new("scalar_count_contract_filter_repair");
    fs::create_dir_all(root.path.join("child")).expect("create child dir");
    fs::write(root.path.join("a.txt"), "a").expect("write file");
    let root_path = root.path.display().to_string();
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.path.clone();
    state.skill_rt.default_locator_search_dir = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = root_path.clone();
    route.output_contract.delivery_required = false;
    route.output_contract.self_extension.scalar_count_filter = crate::OutputScalarCountFilter {
        target_kind: crate::OutputScalarCountTargetKind::Dir,
        include_hidden: Some(false),
        recursive: Some(false),
        extensions: Vec::new(),
    };
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "count_entries",
            "path": root_path.clone(),
            "include_hidden": false
        }),
    }];
    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &LoopState::new(1),
        "count workspace directories",
        None,
        Some(&root_path),
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "count_entries");
    assert_eq!(args.get("kind_filter").and_then(Value::as_str), Some("dir"));
    assert_eq!(args.get("count_dirs").and_then(Value::as_bool), Some(true));
    assert_eq!(
        args.get("count_files").and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(args.get("dirs_only").and_then(Value::as_bool), Some(true));
    assert_eq!(args.get("files_only").and_then(Value::as_bool), Some(false));
    assert_eq!(
        args.get("include_hidden").and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(args.get("recursive").and_then(Value::as_bool), Some(false));
}

#[test]
fn scalar_count_listing_plan_preserves_files_kind_for_count_inventory() {
    let root = TempDirGuard::new("scalar_count_files_only_locator_dir");
    fs::write(root.path.join("a.txt"), "a").expect("write a");
    fs::create_dir_all(root.path.join("child")).expect("create child");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_path.clone();
    route.output_contract.delivery_required = false;
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "count_entries",
            "path": root_path.clone(),
            "kind": "files",
        }),
    }];

    let normalized = replace_scalar_count_plan_with_count_inventory(
        Some(&route),
        &LoopState::new(1),
        Some(&root_path),
        "count files",
        actions,
    );

    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallTool { tool: skill, args } => {
            assert_eq!(skill, "fs_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("count_entries")
            );
            assert_eq!(
                args.get("kind_filter").and_then(Value::as_str),
                Some("file")
            );
            assert_eq!(args.get("count_files").and_then(Value::as_bool), Some(true));
            assert_eq!(args.get("count_dirs").and_then(Value::as_bool), Some(false));
            assert_eq!(args.get("files_only").and_then(Value::as_bool), Some(true));
        }
        other => panic!("expected fs_basic count_entries action, got {other:?}"),
    }
}

#[test]
fn scalar_count_listing_plan_preserves_extension_filter_for_count_inventory() {
    let root = TempDirGuard::new("scalar_count_ext_filter_locator_dir");
    fs::write(root.path.join("a.md"), "a").expect("write a");
    fs::write(root.path.join("b.txt"), "b").expect("write b");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_path.clone();
    route.output_contract.delivery_required = false;
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "count_entries",
            "path": root_path.clone(),
            "ext_filter": "md",
        }),
    }];

    let normalized = replace_scalar_count_plan_with_count_inventory(
        Some(&route),
        &LoopState::new(1),
        Some(&root_path),
        "count markdown files",
        actions,
    );

    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallTool { tool: skill, args } => {
            assert_eq!(skill, "fs_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("count_entries")
            );
            assert_eq!(
                args.get("kind_filter").and_then(Value::as_str),
                Some("file")
            );
            assert_eq!(args.get("ext_filter").and_then(Value::as_str), Some("md"));
        }
        other => panic!("expected fs_basic count_entries action, got {other:?}"),
    }
}

#[test]
fn scalar_count_repair_preserves_explicit_count_path_over_auto_locator() {
    let root = TempDirGuard::new("scalar_count_explicit_over_auto_locator");
    fs::create_dir_all(root.path.join(".git")).expect("create .git");
    fs::create_dir_all(root.path.join("crates")).expect("create crates");
    let root_path = root.path.display().to_string();
    let git_path = root.path.join(".git").display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.delivery_required = false;
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "count_entries",
                "path": root_path.clone(),
                "include_hidden": false,
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = replace_scalar_count_plan_with_count_inventory(
        Some(&route),
        &LoopState::new(1),
        Some(&git_path),
        "count top-level entries except the hidden git directory",
        actions,
    );

    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallTool { tool: skill, args } => {
            assert_eq!(skill, "fs_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("count_entries")
            );
            assert_eq!(
                args.get("path").and_then(Value::as_str),
                Some(root_path.as_str())
            );
            assert_ne!(
                args.get("path").and_then(Value::as_str),
                Some(git_path.as_str())
            );
            assert_eq!(
                args.get("include_hidden").and_then(Value::as_bool),
                Some(false)
            );
        }
        other => panic!("expected fs_basic count_entries action, got {other:?}"),
    }
}

#[test]
fn scalar_count_unqualified_listing_plan_forces_structured_count_repair() {
    let root = TempDirGuard::new("scalar_count_unqualified_listing");
    fs::write(root.path.join("a.txt"), "a").expect("write a");
    fs::create_dir_all(root.path.join("child")).expect("create child");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = root_path.clone();
    route.output_contract.delivery_required = false;
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "list_dir",
                "path": root_path.clone(),
                "names_only": true,
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = replace_scalar_count_plan_with_count_inventory(
        Some(&route),
        &LoopState::new(1),
        Some(&root_path),
        "count entries",
        actions,
    );

    assert_eq!(normalized.len(), 3);
    match &normalized[0] {
        AgentAction::CallTool { tool: skill, args } => {
            assert_eq!(skill, "fs_basic");
            assert_eq!(args.get("action").and_then(Value::as_str), Some("list_dir"));
        }
        other => panic!("expected preserved fs_basic list_dir action, got {other:?}"),
    }
    let state = test_state();
    let loop_state = LoopState::new(1);
    assert!(should_force_actionable_plan_repair(
        &state,
        Some(&route),
        &loop_state,
        &normalized
    ));
    assert_eq!(
        plan_repair_reason(&state, Some(&route), &loop_state, Some(&normalized)),
        "scalar_count_requires_structured_count_action"
    );
}

#[test]
fn scalar_count_missing_explicit_path_checks_that_path_not_auto_parent() {
    let root = TempDirGuard::new("scalar_count_missing_explicit_path");
    let parent = root.path.join("configs");
    fs::create_dir_all(&parent).expect("create parent");
    let parent_path = parent.display().to_string();
    let missing = root.path.join("configs/config_copy");
    let missing_path = missing.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = missing_path.clone();
    route.output_contract.delivery_required = false;
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "inventory_dir",
            "path": missing_path.clone(),
            "ext_filter": "toml"
        }),
    }];

    let normalized = replace_scalar_count_plan_with_count_inventory(
        Some(&route),
        &LoopState::new(1),
        Some(&parent_path),
        "查一下目录下有几个 toml 文件",
        actions,
    );

    assert_eq!(normalized.len(), 2);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            assert_eq!(skill, "fs_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("stat_paths")
            );
            assert_eq!(args.get("paths"), Some(&json!([missing_path])));
        }
        other => panic!("expected fs_basic stat_paths action, got {other:?}"),
    }
    match &normalized[1] {
        AgentAction::Respond { content } => {
            assert!(content.contains("不存在"));
            assert!(content.contains("无法统计"));
        }
        other => panic!("expected missing-path Respond action, got {other:?}"),
    }
}

#[test]
fn observed_missing_read_file_reply_does_not_force_plan_repair() {
    use crate::executor::{StepExecutionResult, StepExecutionStatus};

    let missing_path =
        "/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/docs/missing.md";
    let mut loop_state = LoopState::new(2);
    loop_state.round_no = 2;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "read_file".to_string(),
        status: StepExecutionStatus::Error,
        output: None,
        error: Some(format!("__RC_READ_FILE_NOT_FOUND__:{missing_path}")),
        started_at: 1,
        finished_at: 2,
    });
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.resolved_intent = format!("读取 {missing_path}；如果不存在，只回答“不存在”和这个路径");
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = missing_path.to_string();
    let actions = vec![AgentAction::Respond {
        content: format!("不存在\n{missing_path}"),
    }];

    assert!(!should_force_actionable_plan_repair(
        &test_state(),
        Some(&route),
        &loop_state,
        &actions
    ));
}

#[test]
fn scalar_count_pathlike_hint_in_current_workspace_does_not_use_parent_auto_locator() {
    let root = TempDirGuard::new("scalar_count_pathlike_current_workspace");
    let parent = root.path.join("configs");
    fs::create_dir_all(&parent).expect("create parent");
    let parent_path = parent.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = "configs/config_copy".to_string();
    route.output_contract.delivery_required = false;
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "inventory_dir",
            "path": "configs/config_copy",
            "ext_filter": "toml"
        }),
    }];

    let normalized = replace_scalar_count_plan_with_count_inventory(
        Some(&route),
        &LoopState::new(1),
        Some(&parent_path),
        "查一下目录下有几个 toml 文件",
        actions,
    );

    assert_eq!(normalized.len(), 2);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            assert_eq!(skill, "fs_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("stat_paths")
            );
            assert_eq!(args.get("paths"), Some(&json!(["configs/config_copy"])));
        }
        other => panic!("expected fs_basic stat_paths action, got {other:?}"),
    }
    match &normalized[1] {
        AgentAction::Respond { content } => {
            assert!(content.contains("不存在"));
            assert!(content.contains("无法统计"));
        }
        other => panic!("expected missing-path Respond action, got {other:?}"),
    }
}

#[test]
fn hidden_entries_scalar_contract_uses_inventory_dir() {
    let root = TempDirGuard::new("hidden_entries_scalar_plan");
    fs::write(root.path.join(".env"), "a").expect("write hidden");
    fs::write(root.path.join("visible.txt"), "b").expect("write visible");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::HiddenEntriesCheck;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = root_path.clone();
    route.output_contract.delivery_required = false;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "list_dir".to_string(),
            args: json!({"path": root_path.clone()}),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let state = test_state_with_enabled_skills(&["system_basic", "list_dir"]);
    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &LoopState::new(1),
        "current workspace hidden entries check",
        None,
        Some(&root_path),
        actions,
    );

    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            assert_eq!(skill, "fs_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("list_dir")
            );
            assert_eq!(
                args.get("path").and_then(|value| value.as_str()),
                Some(root_path.as_str())
            );
            assert_eq!(
                args.get("include_hidden").and_then(Value::as_bool),
                Some(true)
            );
            assert_eq!(args.get("names_only").and_then(Value::as_bool), Some(true));
        }
        other => panic!("expected system_basic inventory_dir action, got {other:?}"),
    }
}

#[test]
fn hidden_entries_strict_contract_uses_hidden_inventory_dir() {
    let root = TempDirGuard::new("hidden_entries_strict_plan");
    fs::write(root.path.join(".env"), "a").expect("write hidden");
    fs::write(root.path.join("visible.txt"), "b").expect("write visible");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::HiddenEntriesCheck;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = root_path.clone();
    route.output_contract.delivery_required = false;
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "list_dir",
            "path": root_path.clone(),
            "include_hidden": false,
            "files_only": true,
            "names_only": true,
            "max_entries": 3,
        }),
    }];

    let state = test_state_with_enabled_skills(&["fs_basic"]);
    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &LoopState::new(1),
        "current workspace hidden entries check",
        None,
        Some(&root_path),
        actions,
    );

    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            assert_eq!(skill, "fs_basic");
            assert_eq!(args.get("action").and_then(Value::as_str), Some("list_dir"));
            assert_eq!(
                args.get("path").and_then(Value::as_str),
                Some(root_path.as_str())
            );
            assert_eq!(
                args.get("include_hidden").and_then(Value::as_bool),
                Some(true)
            );
            assert_eq!(args.get("names_only").and_then(Value::as_bool), Some(true));
            assert!(args.get("files_only").is_none());
        }
        other => panic!("expected fs_basic hidden inventory action, got {other:?}"),
    }
}

#[test]
fn hidden_entries_scalar_current_workspace_hint_falls_back_to_dot_inventory() {
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::HiddenEntriesCheck;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = "current directory".to_string();
    route.output_contract.delivery_required = false;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({"command": "find . -maxdepth 1 -name '.*' | wc -l"}),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
    ];

    let state = test_state_with_enabled_skills(&["system_basic", "run_cmd"]);
    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &LoopState::new(1),
        "count hidden entries in current directory",
        None,
        None,
        actions,
    );

    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            assert_eq!(skill, "fs_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("list_dir")
            );
            assert_eq!(args.get("path").and_then(|value| value.as_str()), Some("."));
            assert_eq!(
                args.get("include_hidden").and_then(Value::as_bool),
                Some(true)
            );
        }
        other => panic!("expected system_basic inventory_dir action, got {other:?}"),
    }
}

#[test]
fn service_status_contract_rewrites_pgrep_run_cmd_to_service_control_status() {
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({"command": "pgrep -x telegramd > /dev/null && echo 'running' || echo 'not running'"}),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
    ];

    let normalized = rewrite_service_status_plan_to_service_control(Some(&route), false, actions);

    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "service_control");
            assert_eq!(args.get("action").and_then(Value::as_str), Some("status"));
            assert_eq!(
                args.get("target").and_then(Value::as_str),
                Some("telegramd")
            );
            assert!(args.get("manager_type").is_none());
        }
        other => panic!("expected service_control status action, got {other:?}"),
    }
    assert_eq!(normalized.len(), 1);
}

#[test]
fn service_status_contract_rewrites_pgrep_script_without_trailing_shell_words() {
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": "pgrep -fa telegramd 2>/dev/null; if [ $? -ne 0 ]; then echo 'telegramd is NOT currently running'; else echo 'telegramd is currently running'; fi"}),
    }];

    let normalized = rewrite_service_status_plan_to_service_control(Some(&route), false, actions);

    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "service_control");
            assert_eq!(args.get("action").and_then(Value::as_str), Some("status"));
            assert_eq!(
                args.get("target").and_then(Value::as_str),
                Some("telegramd")
            );
        }
        other => panic!("expected service_control status action, got {other:?}"),
    }
}

#[test]
fn service_status_contract_rewrites_systemctl_status_to_service_control_systemd() {
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "run_cmd",
            "command": "systemctl is-active nginx.service"
        }),
    }];

    let normalized = rewrite_service_status_plan_to_service_control(Some(&route), false, actions);

    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "service_control");
            assert_eq!(
                args.get("target").and_then(Value::as_str),
                Some("nginx.service")
            );
            assert_eq!(
                args.get("manager_type").and_then(Value::as_str),
                Some("systemd")
            );
        }
        other => panic!("expected service_control status action, got {other:?}"),
    }
}

#[test]
fn normalize_prefers_registry_repair_over_legacy_service_rewrite() {
    let state = test_state_with_registry();
    let loop_state = LoopState::new(1);
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": "systemctl status clawd"}),
    }];

    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &loop_state,
        "check clawd service status",
        None,
        None,
        actions,
    );

    assert!(matches!(
        &normalized[0],
        AgentAction::CallSkill { skill, .. } if skill == "run_cmd"
    ));
    assert!(should_force_actionable_plan_repair(
        &state,
        Some(&route),
        &loop_state,
        &normalized
    ));
    assert_eq!(
        plan_repair_reason(&state, Some(&route), &loop_state, Some(&normalized)),
        "preferred_skill_required_for_semantic_route"
    );
}

#[test]
fn normalize_prefers_registry_sqlite_rewrite_over_text_read_fallback() {
    let state = test_state_with_registry();
    let loop_state = LoopState::new(1);
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::SqliteTableListing;
    route.output_contract.locator_hint = "data/db-basic-contract.sqlite".to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "read_file".to_string(),
        args: json!({"path": "data/db-basic-contract.sqlite"}),
    }];

    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &loop_state,
        "list sqlite tables",
        None,
        Some("data/db-basic-contract.sqlite"),
        actions,
    );

    assert!(planned_call_is(&normalized[0], "db_basic", "list_tables"));
    assert!(!should_force_actionable_plan_repair(
        &state,
        Some(&route),
        &loop_state,
        &normalized
    ));
}

#[test]
fn normalize_prefers_registry_repair_over_legacy_docker_rewrite() {
    let state = test_state_with_registry();
    let loop_state = LoopState::new(1);
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::DockerPs;
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": "docker ps"}),
    }];

    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &loop_state,
        "show docker containers",
        None,
        None,
        actions,
    );

    assert!(matches!(
        &normalized[0],
        AgentAction::CallSkill { skill, .. } if skill == "run_cmd"
    ));
    assert_eq!(
        plan_repair_reason(&state, Some(&route), &loop_state, Some(&normalized)),
        "preferred_skill_required_for_semantic_route"
    );
}

#[test]
fn archive_unpack_semantic_kind_without_capability_ref_stays_non_actionable() {
    let state = test_state_with_registry();
    let loop_state = LoopState::new(1);
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ArchiveUnpack;
    route.output_contract.locator_hint = "/tmp/source.tgz | /tmp/source-unpacked".to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": "tar -xzf /tmp/source.tgz -C /tmp/source-unpacked"}),
    }];

    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &loop_state,
        "unpack archive",
        None,
        None,
        actions,
    );

    assert!(matches!(
        &normalized[0],
        AgentAction::CallSkill { skill, .. } if skill == "run_cmd"
    ));
    assert_eq!(
        plan_repair_reason(&state, Some(&route), &loop_state, Some(&normalized)),
        "non_actionable_plan_for_current_route"
    );
}

#[test]
fn explicit_service_command_is_preserved_as_run_cmd() {
    let mut state = test_state_with_enabled_skills(&["service_control", "run_cmd"]);
    state.policy.command_intent.execute_prefixes = vec!["执行命令 ".to_string()];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    let actions = vec![AgentAction::CallSkill {
        skill: "service_control".to_string(),
        args: json!({
            "action": "status",
            "target": "clawd"
        }),
    }];

    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &LoopState::new(1),
        "执行命令 systemctl status clawd --no-pager，告诉我结果",
        Some("执行命令 systemctl status clawd --no-pager，告诉我结果"),
        None,
        actions,
    );

    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "run_cmd");
            assert_eq!(
                args.get("command").and_then(Value::as_str),
                Some("systemctl status clawd --no-pager")
            );
        }
        other => panic!("expected preserved run_cmd action, got {other:?}"),
    }
}

#[test]
fn observed_judgment_mixed_placeholder_respond_uses_synthesize_after_listing() {
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RecentArtifactsJudgment;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "document".to_string();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "list_dir".to_string(),
            args: json!({"path": "document", "limit": 5}),
        },
        AgentAction::Respond {
            content:
                "Here are the first files:\n{{last_output}}\nThese look more like documentation."
                    .to_string(),
        },
    ];

    let state = test_state_with_enabled_skills(&["list_dir"]);
    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &LoopState::new(1),
        "list files and judge their role",
        None,
        None,
        actions,
    );

    assert_eq!(normalized.len(), 3);
    assert!(planned_call_is(&normalized[0], "fs_basic", "list_dir"));
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
fn scalar_count_preserves_planned_run_cmd_observation() {
    let root = TempDirGuard::new("scalar_count_run_cmd_plan");
    fs::write(root.path.join(".env"), "a").expect("write hidden");
    fs::write(root.path.join("visible.txt"), "b").expect("write visible");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = root_path.clone();
    route.output_contract.delivery_required = false;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({"command": "printf '2\\n'"}),
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let state = test_state_with_enabled_skills(&["system_basic", "run_cmd"]);
    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &LoopState::new(1),
        "count current workspace entries",
        None,
        Some(&root_path),
        actions,
    );

    match normalized
        .iter()
        .find(|action| matches!(action, AgentAction::CallSkill { skill, .. } if skill == "run_cmd"))
    {
        Some(AgentAction::CallSkill { skill, args }) => {
            assert_eq!(skill, "run_cmd");
            assert_eq!(
                args.get("command").and_then(|value| value.as_str()),
                Some("printf '2\\n'")
            );
        }
        other => panic!("expected run_cmd action, got {other:?}"),
    }
    assert!(!normalized.iter().any(|action| {
        matches!(
            action,
            AgentAction::CallSkill { skill, args }
                if skill == "system_basic"
                    && args.get("action").and_then(Value::as_str) == Some("count_inventory")
        )
    }));
}

#[test]
fn structured_keys_contract_rewrites_read_range_to_structured_keys() {
    let root = TempDirGuard::new("structured_keys_plan");
    let config_path = root.path.join("config.toml");
    fs::write(&config_path, "alpha = 1\n[beta]\nvalue = 2\n").expect("write config");
    let config_path = config_path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::StructuredKeys;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = config_path.clone();
    route.output_contract.delivery_required = false;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({"action": "read_range", "path": config_path.clone()}),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let state = test_state_with_enabled_skills(&["system_basic"]);
    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &LoopState::new(1),
        "list structured keys",
        None,
        Some(&config_path),
        actions,
    );

    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            assert_eq!(skill, "config_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("list_keys")
            );
            assert_eq!(
                args.get("path").and_then(Value::as_str),
                Some(config_path.as_str())
            );
        }
        other => panic!("expected config_basic list_keys action, got {other:?}"),
    }
}

#[test]
fn structured_keys_contract_rewrites_validate_to_structured_keys() {
    let root = TempDirGuard::new("structured_keys_validate_plan");
    let config_path = root.path.join("config.toml");
    fs::write(&config_path, "alpha = 1\n[beta]\nvalue = 2\n").expect("write config");
    let config_path = config_path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::StructuredKeys;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = config_path.clone();
    let actions = vec![AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args: json!({
            "action": "validate",
            "path": config_path.clone(),
            "format": "toml",
            "validation_profile": "syntax_only",
        }),
    }];

    let state = test_state_with_enabled_skills(&["config_basic"]);
    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &LoopState::new(1),
        "list structured keys",
        None,
        Some(&config_path),
        actions,
    );

    assert_eq!(normalized.len(), 1);
    let args = expect_planned_call(&normalized[0], "config_basic", "list_keys");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(config_path.as_str())
    );
}

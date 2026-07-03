use super::*;

fn assert_empty_planner_actions_stay_empty(
    route: &RouteResult,
    loop_state: &LoopState,
    goal: &str,
    user_text: Option<&str>,
    auto_locator_path: Option<&str>,
) {
    let state = test_state();
    let normalized = normalize_planned_actions_with_original_and_context(
        &state,
        Some(route),
        loop_state,
        goal,
        user_text,
        None,
        auto_locator_path,
        vec![],
    );
    assert!(
        normalized.is_empty(),
        "runtime must not inject a pre-LLM deterministic directory plan: {normalized:?}"
    );
}

#[test]
fn file_paths_directory_locator_builds_structured_list_dir_plan() {
    let root = TempDirGuard::new("file_paths_directory_locator");
    fs::write(root.path.join("lib.rs"), "fn planner_loop_marker() {}\n").expect("write rust");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::FilePaths;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_path.clone();
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
    route.output_contract.delivery_required = false;
    route.route_reason = "capability_ref=filesystem.list_dir".to_string();
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let args = assert_planner_supplied_tool_call_preserved(
        &test_state(),
        &route,
        &loop_state,
        "list the largest files under a directory",
        Some("list the largest files under a directory"),
        None,
        "fs_basic",
        "list_dir",
        json!({
            "action": "list_dir",
            "path": root_path.clone(),
            "files_only": true,
            "max_entries": 3,
            "sort_by": "size_desc",
        }),
    );

    assert_eq!(args.get("action").and_then(Value::as_str), Some("list_dir"));
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(root_path.as_str())
    );
    assert_eq!(args.get("files_only").and_then(Value::as_bool), Some(true));
    assert_eq!(args.get("max_entries").and_then(Value::as_u64), Some(3));
    assert_eq!(
        args.get("sort_by").and_then(Value::as_str),
        Some("size_desc")
    );
}

#[test]
fn file_paths_directory_locator_with_extension_token_uses_recursive_find_entries() {
    let root = TempDirGuard::new("file_paths_directory_locator_ext");
    fs::write(root.path.join("Cargo.toml"), "[package]\nname='fixture'\n").expect("write toml");
    fs::write(root.path.join("Cargo.lock"), "# not toml").expect("write lock");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::FilePaths;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_path.clone();
    route.resolved_intent =
        "Find all TOML files in this repository and mention representative ones".to_string();
    route.output_contract.delivery_required = false;
    route.route_reason = "capability_ref=filesystem.find_entries".to_string();
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let args = assert_planner_supplied_tool_call_preserved(
        &test_state(),
        &route,
        &loop_state,
        "find toml files in this repo and briefly mention a few representative ones",
        Some("find toml files in this repo and briefly mention a few representative ones"),
        None,
        "fs_basic",
        "find_entries",
        json!({
            "action": "find_entries",
            "root": root_path.clone(),
            "ext": "toml",
            "target_kind": "file",
            "recursive": true,
        }),
    );

    assert_eq!(
        args.get("action").and_then(Value::as_str),
        Some("find_entries")
    );
    assert_eq!(
        args.get("root").and_then(Value::as_str),
        Some(root_path.as_str())
    );
    assert_eq!(args.get("ext").and_then(Value::as_str), Some("toml"));
    assert_eq!(
        args.get("target_kind").and_then(Value::as_str),
        Some("file")
    );
    assert_eq!(args.get("recursive").and_then(Value::as_bool), Some(true));
}

#[test]
fn scalar_path_auto_locator_does_not_use_deterministic_plan_for_directory_search_scope() {
    let root = TempDirGuard::new("scalar_auto_locator_search_scope");
    fs::write(root.path.join("ABCD.txt"), "hello").expect("write report");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_path.clone();
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    assert_empty_planner_actions_stay_empty(
        &route,
        &loop_state,
        "find a named item inside the resolved directory",
        Some("find a named item inside the resolved directory"),
        Some(&root_path),
    );
}

#[test]
fn scalar_path_directory_locator_search_uses_structural_name_target() {
    let root = TempDirGuard::new("scalar_auto_locator_search_target");
    fs::write(root.path.join("ABCD.txt"), "hello").expect("write report");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_path.clone();
    route.output_contract.delivery_required = false;
    route.route_reason = "capability_ref=filesystem.find_entries".to_string();
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let args = assert_planner_supplied_tool_call_preserved(
        &test_state(),
        &route,
        &loop_state,
        "find a named item inside the resolved directory",
        Some(&format!("去 {root_path} 找 abcd，只输出路径")),
        None,
        "fs_basic",
        "find_entries",
        json!({
            "action": "find_entries",
            "root": root_path.clone(),
            "pattern": "abcd",
            "target_kind": "any",
        }),
    );

    assert_eq!(
        args.get("action").and_then(Value::as_str),
        Some("find_entries")
    );
    assert_eq!(
        args.get("root").and_then(Value::as_str),
        Some(root_path.as_str())
    );
    assert_eq!(args.get("pattern").and_then(Value::as_str), Some("abcd"));
    assert_eq!(args.get("target_kind").and_then(Value::as_str), Some("any"));
}

#[test]
fn scalar_path_directory_locator_search_resolves_unique_entry_token_without_phrase_matching() {
    let root = TempDirGuard::new("scalar_auto_locator_search_unique_token");
    fs::write(root.path.join("ABCD.txt"), "hello").expect("write target");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_path.clone();
    route.output_contract.delivery_required = false;
    route.route_reason = "capability_ref=filesystem.find_entries".to_string();
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let args = assert_planner_supplied_tool_call_preserved(
        &test_state(),
        &route,
        &loop_state,
        "find a named item inside the resolved directory",
        Some(&format!(
            "Inside {root_path}, find abcd and return only the path"
        )),
        None,
        "fs_basic",
        "find_entries",
        json!({
            "action": "find_entries",
            "root": root_path.clone(),
            "pattern": "abcd",
            "target_kind": "any",
        }),
    );

    assert_eq!(
        args.get("action").and_then(Value::as_str),
        Some("find_entries")
    );
    assert_eq!(args.get("pattern").and_then(Value::as_str), Some("abcd"));
}

#[test]
fn scalar_path_directory_locator_search_rejects_ambiguous_current_quoted_targets() {
    let root = TempDirGuard::new("scalar_auto_locator_search_ambiguous_quotes");
    fs::write(root.path.join("ABCD.txt"), "hello").expect("write first target");
    fs::write(root.path.join("WXYZ.txt"), "hello").expect("write second target");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_path.clone();
    route.output_contract.delivery_required = false;
    route.resolved_intent = r#"find "ABCD""#.to_string();
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    assert_empty_planner_actions_stay_empty(
        &route,
        &loop_state,
        "find a named item inside the resolved directory",
        Some(&format!(
            r#"Inside {root_path}, find "ABCD" or "WXYZ" and return only the path"#
        )),
        Some(&root_path),
    );
}

#[test]
fn scalar_path_directory_locator_search_requires_scalar_path_contract() {
    let root = TempDirGuard::new("scalar_auto_locator_search_requires_contract");
    fs::write(root.path.join("ABCD.txt"), "hello").expect("write target");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_path.clone();
    route.output_contract.delivery_required = false;

    assert!(scalar_path_directory_locator_search_observation_plan(
        Some(&route),
        Some(&root_path),
        &format!("Inside {root_path}, find abcd and return only the path"),
    )
    .is_none());
}

#[test]
fn scalar_path_auto_locator_directory_builds_observation_plan() {
    let root = TempDirGuard::new("scalar_auto_locator_dir");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.delivery_required = false;

    let actions =
        scalar_path_auto_locator_observation_plan(Some(&route), Some(&root_path)).unwrap();
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            assert_eq!(skill, "fs_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("stat_paths")
            );
            assert_eq!(args.get("paths"), Some(&json!([root_path])));
        }
        other => panic!("expected fs_basic stat_paths action, got {other:?}"),
    }
}

#[test]
fn generic_directory_auto_locator_builds_inventory_synthesis_plan() {
    let root = TempDirGuard::new("generic_dir_auto_locator");
    fs::write(root.path.join("small.log"), "x").expect("write small");
    fs::write(root.path.join("large.log"), "xxxxxx").expect("write large");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = String::new();
    route.output_contract.delivery_required = false;

    let actions =
        generic_directory_auto_locator_observation_plan(Some(&route), Some(root_path.as_str()))
            .expect("directory route should build a default observation plan");

    assert_eq!(actions.len(), 3);
    match &actions[0] {
        AgentAction::CallTool { tool, args } => {
            assert_eq!(tool, "fs_basic");
            assert_eq!(args.get("action").and_then(Value::as_str), Some("list_dir"));
            assert_eq!(
                args.get("path").and_then(Value::as_str),
                Some(root_path.as_str())
            );
            assert_eq!(
                args.get("sort_by").and_then(Value::as_str),
                Some("size_desc")
            );
            assert_eq!(args.get("names_only").and_then(Value::as_bool), Some(false));
        }
        other => panic!("expected fs_basic list_dir action, got {other:?}"),
    }
    assert!(matches!(actions[1], AgentAction::SynthesizeAnswer { .. }));
    assert!(matches!(actions[2], AgentAction::Respond { .. }));
}

#[test]
fn directory_entry_groups_auto_locator_uses_fs_basic_list_dir() {
    let root = TempDirGuard::new("directory_entry_groups_auto_locator");
    fs::create_dir_all(root.path.join("docs")).expect("create docs");
    fs::write(root.path.join("README.md"), "hello").expect("write readme");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::direct_answer(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryEntryGroups;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_path.clone();
    route.route_reason = "capability_ref=filesystem.list_dir".to_string();

    let args = assert_planner_supplied_tool_call_preserved(
        &test_state(),
        &route,
        &LoopState::new(1),
        "group directory entries",
        Some("按文件和文件夹分组"),
        None,
        "fs_basic",
        "list_dir",
        json!({
            "action": "list_dir",
            "path": root_path.clone(),
            "names_only": false,
            "sort_by": "mtime_desc",
        }),
    );

    assert_eq!(args.get("action").and_then(Value::as_str), Some("list_dir"));
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(root_path.as_str())
    );
    assert_eq!(args.get("names_only").and_then(Value::as_bool), Some(false));
    assert_eq!(
        args.get("sort_by").and_then(Value::as_str),
        Some("mtime_desc")
    );
}

#[test]
fn directory_entry_groups_auto_locator_preserves_bounded_names_shape() {
    let root = TempDirGuard::new("directory_entry_groups_auto_locator_bounded");
    fs::create_dir_all(root.path.join("docs")).expect("create docs");
    fs::write(root.path.join("README.md"), "hello").expect("write readme");
    fs::write(root.path.join("Cargo.toml"), "hello").expect("write cargo");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryEntryGroups;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_path.clone();
    route.output_contract.self_extension.list_selector = crate::OutputListSelector {
        target_kind: crate::OutputScalarCountTargetKind::Any,
        target_kind_specified: false,
        limit: Some(4),
        sort_by: Some("name".to_string()),
        include_metadata: Some(false),
        include_hidden: None,
    };
    route.resolved_intent =
        "legacy first-layer summary selector_limit=9 selector_sort_by=mtime_desc".to_string();
    route.route_reason = "capability_ref=filesystem.list_dir".to_string();

    let args = assert_planner_supplied_tool_call_preserved(
        &test_state(),
        &route,
        &LoopState::new(1),
        "list bounded entry names",
        Some("list bounded entry names"),
        None,
        "fs_basic",
        "list_dir",
        json!({
            "action": "list_dir",
            "path": root_path.clone(),
            "names_only": true,
            "max_entries": 4,
            "sort_by": "name",
        }),
    );

    assert_eq!(args.get("names_only").and_then(Value::as_bool), Some(true));
    assert_eq!(args.get("max_entries").and_then(Value::as_u64), Some(4));
    assert_eq!(args.get("sort_by").and_then(Value::as_str), Some("name"));
}

#[test]
fn directory_entry_groups_auto_locator_preserves_name_desc_selector() {
    let root = TempDirGuard::new("directory_entry_groups_auto_locator_name_desc");
    fs::create_dir_all(root.path.join("docs")).expect("create docs");
    fs::write(root.path.join("README.md"), "hello").expect("write readme");
    fs::write(root.path.join("Cargo.toml"), "hello").expect("write cargo");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryEntryGroups;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_path.clone();
    route.output_contract.self_extension.list_selector = crate::OutputListSelector {
        target_kind: crate::OutputScalarCountTargetKind::Any,
        target_kind_specified: false,
        limit: Some(5),
        sort_by: Some("name_desc".to_string()),
        include_metadata: Some(false),
        include_hidden: None,
    };
    route.route_reason = "capability_ref=filesystem.list_dir".to_string();

    let args = assert_planner_supplied_tool_call_preserved(
        &test_state(),
        &route,
        &LoopState::new(1),
        "list bounded entry names",
        Some("list bounded entry names"),
        None,
        "fs_basic",
        "list_dir",
        json!({
            "action": "list_dir",
            "path": root_path.clone(),
            "names_only": true,
            "max_entries": 5,
            "sort_by": "name_desc",
        }),
    );

    assert_eq!(args.get("names_only").and_then(Value::as_bool), Some(true));
    assert_eq!(args.get("max_entries").and_then(Value::as_u64), Some(5));
    assert_eq!(
        args.get("sort_by").and_then(Value::as_str),
        Some("name_desc")
    );
}

#[test]
fn generic_directory_auto_locator_uses_mtime_for_directory_entry_groups() {
    let root = TempDirGuard::new("generic_dir_entry_group_auto_locator");
    fs::create_dir_all(root.path.join("docs")).expect("create docs");
    fs::write(root.path.join("README.md"), "hello").expect("write readme");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryEntryGroups;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_path.clone();

    let actions =
        generic_directory_auto_locator_observation_plan(Some(&route), Some(root_path.as_str()))
            .expect("directory entry group fallback should build an observation plan");

    assert_eq!(actions.len(), 3);
    match &actions[0] {
        AgentAction::CallTool { tool, args } => {
            assert_eq!(tool, "fs_basic");
            assert_eq!(args.get("action").and_then(Value::as_str), Some("list_dir"));
            assert_eq!(
                args.get("path").and_then(Value::as_str),
                Some(root_path.as_str())
            );
            assert_eq!(
                args.get("sort_by").and_then(Value::as_str),
                Some("mtime_desc")
            );
            assert_eq!(args.get("names_only").and_then(Value::as_bool), Some(false));
        }
        other => panic!("expected fs_basic list_dir action, got {other:?}"),
    }
}

#[test]
fn directory_entry_groups_preserves_tree_summary_action() {
    let root = TempDirGuard::new("directory_entry_groups_rewrite");
    fs::create_dir_all(root.path.join("docs")).expect("create docs");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryEntryGroups;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_path.clone();
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "tree_summary",
            "path": root_path,
            "max_depth": 2
        }),
    }];

    let rewritten =
        rewrite_directory_entry_groups_tree_summary_to_list_dir(Some(&route), None, actions);

    assert_eq!(rewritten.len(), 1);
    assert!(matches!(
        &rewritten[0],
        AgentAction::CallSkill { skill, args }
            if skill == "system_basic"
                && args.get("action").and_then(Value::as_str) == Some("tree_summary")
    ));
}

#[test]
fn directory_entry_groups_auto_locator_uses_tree_summary_for_machine_action_token() {
    let root = TempDirGuard::new("directory_entry_groups_tree_summary_token");
    fs::create_dir_all(root.path.join("docs")).expect("create docs");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryEntryGroups;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_path.clone();
    route.output_contract.delivery_required = false;
    route.route_reason = "capability_ref=system.tree_summary".to_string();
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let args = assert_planner_supplied_skill_call_preserved(
        &test_state(),
        &route,
        &loop_state,
        "goal",
        Some("inspect with tree_summary"),
        None,
        "system_basic",
        "tree_summary",
        json!({
            "action": "tree_summary",
            "path": root_path.clone(),
        }),
    );

    assert_eq!(
        args.get("action").and_then(Value::as_str),
        Some("tree_summary")
    );
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(root_path.as_str())
    );
}

#[test]
fn directory_names_contract_overrides_planner_hidden_inventory() {
    let root = TempDirGuard::new("directory_names_hidden_override");
    fs::create_dir_all(root.path.join(".cache")).expect("create hidden dir");
    fs::create_dir_all(root.path.join("docs")).expect("create docs dir");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryNames;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = root_path.clone();
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "list_dir",
            "path": root_path,
            "dirs_only": true,
            "include_hidden": true,
            "names_only": true
        }),
    }];

    let normalized = normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(1),
        "list directory names except ignored hidden VCS internals",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 1);
    let args = expect_planned_call(&normalized[0], "fs_basic", "list_dir");
    assert_eq!(args.get("dirs_only").and_then(Value::as_bool), Some(true));
    assert_eq!(args.get("names_only").and_then(Value::as_bool), Some(true));
    assert_eq!(
        args.get("include_hidden").and_then(Value::as_bool),
        Some(false)
    );
}

#[test]
fn directory_tree_auto_locator_deterministic_plan_uses_system_basic_tree_summary() {
    let root = TempDirGuard::new("directory_tree_auto_locator");
    fs::create_dir_all(root.path.join("archive")).expect("create archive dir");
    fs::write(root.path.join("archive").join("README.txt"), "archive").expect("write readme");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::direct_answer(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_path.clone();
    route.route_reason = "capability_ref=system.tree_summary".to_string();

    let args = assert_planner_supplied_skill_call_preserved(
        &test_state(),
        &route,
        &LoopState::new(1),
        "summarize directory structure",
        Some("summarize directory structure"),
        None,
        "system_basic",
        "tree_summary",
        json!({
            "action": "tree_summary",
            "path": root_path.clone(),
        }),
    );

    assert_eq!(
        args.get("action").and_then(Value::as_str),
        Some("tree_summary")
    );
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(root_path.as_str())
    );
}

#[test]
fn workspace_summary_auto_locator_lists_structure_and_reads_readme() {
    let root = TempDirGuard::new("workspace_summary_auto_locator");
    fs::create_dir_all(root.path.join("UI")).expect("create UI dir");
    fs::create_dir_all(root.path.join("crates")).expect("create crates dir");
    fs::create_dir_all(root.path.join("scripts")).expect("create scripts dir");
    fs::write(
        root.path.join("README.md"),
        "# Fixture\n\nA local runtime with UI and Rust crates.",
    )
    .expect("write README");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::WorkspaceProjectSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_path.clone();
    route.route_reason =
        "capability_ref=filesystem.list_dir capability_ref=filesystem.read_text_range".to_string();

    let normalized = normalize_planned_actions_with_original_and_context(
        &test_state(),
        Some(&route),
        &LoopState::new(1),
        "summarize workspace structure",
        Some("summarize workspace structure"),
        None,
        None,
        vec![
            AgentAction::CallTool {
                tool: "fs_basic".to_string(),
                args: json!({
                    "action": "list_dir",
                    "path": root_path.clone(),
                    "dirs_only": true,
                    "max_entries": 100,
                    "sort_by": "name",
                }),
            },
            AgentAction::CallTool {
                tool: "fs_basic".to_string(),
                args: json!({
                    "action": "read_text_range",
                    "path": root.path.join("README.md").display().to_string(),
                    "mode": "head",
                    "n": 80,
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

    assert_eq!(normalized.len(), 4);
    let list_idx = normalized
        .iter()
        .position(|action| planned_call_is(action, "fs_basic", "list_dir"))
        .expect("list_dir evidence action");
    let list_args = expect_planned_call(&normalized[list_idx], "fs_basic", "list_dir");
    assert_eq!(
        list_args.get("path").and_then(Value::as_str),
        Some(root_path.as_str())
    );
    assert_eq!(
        list_args.get("dirs_only").and_then(Value::as_bool),
        Some(true)
    );

    let read_idx = normalized
        .iter()
        .position(|action| {
            planned_call(action).is_some_and(|(skill, args)| {
                let action_name = args.get("action").and_then(Value::as_str);
                ((skill == "fs_basic" && action_name == Some("read_text_range"))
                    || (skill == "doc_parse" && action_name == Some("parse_doc")))
                    && args
                        .get("path")
                        .and_then(Value::as_str)
                        .is_some_and(|path| path.ends_with("README.md"))
            })
        })
        .expect("README evidence action");

    assert!(matches!(
        normalized.get(2),
        Some(AgentAction::SynthesizeAnswer { evidence_refs })
            if evidence_refs.contains(&format!("step_{}", list_idx + 1))
                && evidence_refs.contains(&format!("step_{}", read_idx + 1))
    ));
    assert!(matches!(
        normalized.get(3),
        Some(AgentAction::Respond { content }) if content == "{{last_output}}"
    ));
}

#[test]
fn directory_purpose_auto_locator_lists_directory_and_reads_text_candidates() {
    let root = TempDirGuard::new("directory_purpose_auto_locator");
    fs::create_dir_all(root.path.join("docs")).expect("create docs dir");
    fs::write(root.path.join("docs").join("README.txt"), "docs").expect("write readme");
    fs::write(root.path.join("docs").join("image.png"), "not text").expect("write image");
    let docs_path = root.path.join("docs").display().to_string();
    let mut route = route_result(
        crate::AskMode::direct_answer(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryPurposeSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = docs_path.clone();
    route.route_reason =
        "capability_ref=filesystem.list_dir capability_ref=filesystem.read_text_range".to_string();

    let normalized = normalize_planned_actions_with_original_and_context(
        &test_state(),
        Some(&route),
        &LoopState::new(1),
        "summarize directory purpose",
        Some("summarize directory purpose"),
        None,
        None,
        vec![
            AgentAction::CallTool {
                tool: "fs_basic".to_string(),
                args: json!({
                    "action": "list_dir",
                    "path": docs_path.clone(),
                    "names_only": false,
                    "max_entries": 100,
                }),
            },
            AgentAction::CallTool {
                tool: "fs_basic".to_string(),
                args: json!({
                    "action": "read_text_range",
                    "path": root.path.join("docs").join("README.txt").display().to_string(),
                    "mode": "head",
                    "n": 80,
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

    assert_eq!(normalized.len(), 4);
    let list_args = expect_planned_call(&normalized[0], "fs_basic", "list_dir");
    assert_eq!(
        list_args.get("path").and_then(Value::as_str),
        Some(docs_path.as_str())
    );

    let read_args = expect_planned_call(&normalized[1], "fs_basic", "read_text_range");
    assert!(read_args
        .get("path")
        .and_then(Value::as_str)
        .is_some_and(|path| path.ends_with("README.txt")));
    assert!(matches!(
        normalized.get(1),
        Some(AgentAction::CallTool { .. })
    ));
    assert!(matches!(
        normalized.get(2),
        Some(AgentAction::SynthesizeAnswer { evidence_refs })
            if *evidence_refs == vec!["step_1".to_string(), "step_2".to_string()]
    ));
    assert!(matches!(
        normalized.get(3),
        Some(AgentAction::Respond { content }) if content == "{{last_output}}"
    ));

    assert_empty_planner_actions_stay_empty(
        &route,
        &LoopState::new(1),
        "summarize directory purpose",
        Some("summarize directory purpose"),
        Some(&docs_path),
    );
}

#[test]
fn directory_purpose_auto_locator_uses_inventory_for_many_text_candidates() {
    let root = TempDirGuard::new("directory_purpose_many_text_candidates");
    fs::create_dir_all(root.path.join("src")).expect("create src dir");
    for idx in 0..9 {
        fs::write(
            root.path.join(format!("note_{idx}.md")),
            format!("note {idx}"),
        )
        .expect("write note");
    }
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryPurposeSummary;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = root_path.clone();

    let plan = directory_purpose_auto_locator_deterministic_plan_result(
        &test_state(),
        "summarize directory purpose",
        Some(&route),
        &LoopState::new(1),
        "summarize directory purpose",
        Some("summarize directory purpose"),
        Some(&root_path),
    )
    .expect("large directory purpose plan should use bounded inventory");

    assert_eq!(plan.plan_kind, PlanKind::Single);
    assert_eq!(plan.steps.len(), 3);
    let list_action = plan.steps[0].to_agent_action().expect("list action");
    let list_args = expect_planned_call(&list_action, "fs_basic", "list_dir");
    assert_eq!(
        list_args.get("path").and_then(Value::as_str),
        Some(root_path.as_str())
    );
    assert_eq!(
        list_args.get("max_entries").and_then(Value::as_i64),
        Some(1000)
    );
    assert_eq!(
        list_args.get("dirs_only").and_then(Value::as_bool),
        Some(true)
    );
    assert!(matches!(
        plan.steps.get(1).and_then(|step| step.to_agent_action()),
        Some(AgentAction::SynthesizeAnswer { evidence_refs })
            if evidence_refs == vec!["last_output".to_string()]
    ));
    assert!(matches!(
        plan.steps.get(2).and_then(|step| step.to_agent_action()),
        Some(AgentAction::Respond { content }) if content == "{{last_output}}"
    ));
}

#[test]
fn directory_purpose_extension_locator_uses_recursive_find_entries_not_tree_summary() {
    let root = TempDirGuard::new("directory_purpose_extension_locator");
    fs::write(root.path.join("Cargo.toml"), "[workspace]\n").expect("write cargo");
    fs::create_dir_all(root.path.join("configs")).expect("create configs");
    fs::write(root.path.join("configs/config.toml"), "[skills]\n").expect("write config");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::direct_answer(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryPurposeSummary;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = "*.toml".to_string();

    assert!(directory_tree_auto_locator_deterministic_plan_result(
        &test_state(),
        "summarize representative toml files",
        Some(&route),
        &LoopState::new(1),
        "summarize representative toml files",
        Some("summarize representative toml files"),
        Some(&root_path),
    )
    .is_none());

    let plan = directory_purpose_extension_inventory_deterministic_plan_result(
        "summarize representative toml files",
        Some(&route),
        &LoopState::new(1),
        Some(&root_path),
    )
    .expect("directory purpose extension inventory plan");

    assert_eq!(plan.plan_kind, PlanKind::Single);
    assert_eq!(plan.steps.len(), 5);
    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "fs_basic", "find_entries");
    assert_eq!(
        args.get("root").and_then(Value::as_str),
        Some(root_path.as_str())
    );
    assert_eq!(args.get("ext").and_then(Value::as_str), Some("toml"));
    assert_eq!(
        args.get("target_kind").and_then(Value::as_str),
        Some("file")
    );
    assert_eq!(args.get("recursive").and_then(Value::as_bool), Some(true));
    assert_eq!(
        args.get("sort_by").and_then(Value::as_str),
        Some("size_desc")
    );
    let read_action = plan.steps[1].to_agent_action().expect("read action");
    let read_args = expect_planned_call(&read_action, "fs_basic", "read_text_range");
    assert!(read_args
        .get("path")
        .and_then(Value::as_str)
        .is_some_and(|path| path.ends_with("Cargo.toml")));
    assert!(matches!(
        plan.steps.get(3).and_then(|step| step.to_agent_action()),
        Some(AgentAction::SynthesizeAnswer { evidence_refs })
            if evidence_refs == vec![
                "step_1".to_string(),
                "step_2".to_string(),
                "step_3".to_string()
            ]
    ));
    assert!(matches!(
        plan.steps.get(4).and_then(|step| step.to_agent_action()),
        Some(AgentAction::Respond { content }) if content == "{{last_output}}"
    ));
}

#[test]
fn directory_purpose_extension_from_resolved_intent_uses_recursive_find_entries() {
    let root = TempDirGuard::new("directory_purpose_resolved_intent_extension");
    fs::write(root.path.join("intent_normalizer.schema.json"), "{}").expect("write schema");
    fs::create_dir_all(root.path.join("nested")).expect("create nested");
    fs::write(root.path.join("nested/contract_repair.schema.json"), "{}").expect("write nested");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryPurposeSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_path.clone();
    route.resolved_intent =
        "List .json files, find the largest schema, and summarize its purpose.".to_string();

    let plan = directory_purpose_extension_inventory_deterministic_plan_result(
        "summarize json schema directory",
        Some(&route),
        &LoopState::new(1),
        Some(&root_path),
    )
    .expect("directory purpose extension inventory plan");

    assert_eq!(plan.plan_kind, PlanKind::Single);
    assert_eq!(plan.steps.len(), 5);
    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "fs_basic", "find_entries");
    assert_eq!(
        args.get("root").and_then(Value::as_str),
        Some(root_path.as_str())
    );
    assert_eq!(args.get("ext").and_then(Value::as_str), Some("json"));
    assert_eq!(
        args.get("target_kind").and_then(Value::as_str),
        Some("file")
    );
    assert_eq!(args.get("recursive").and_then(Value::as_bool), Some(true));
    assert_eq!(
        args.get("sort_by").and_then(Value::as_str),
        Some("size_desc")
    );
    let read_paths = plan
        .steps
        .iter()
        .filter_map(|step| step.to_agent_action())
        .filter_map(|action| match action {
            AgentAction::CallTool { tool, args } if tool == "fs_basic" => {
                let action_name = args.get("action").and_then(Value::as_str)?;
                (action_name == "read_text_range")
                    .then(|| args.get("path").and_then(Value::as_str).map(str::to_string))
                    .flatten()
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(read_paths
        .iter()
        .any(|path| path.ends_with("intent_normalizer.schema.json")));
    assert!(read_paths
        .iter()
        .any(|path| path.ends_with("nested/contract_repair.schema.json")));
}

#[test]
fn directory_purpose_extension_inventory_defers_explicit_extension_assess_gap() {
    let root = TempDirGuard::new("directory_purpose_extension_assess_gap_defers");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryPurposeSummary;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = "*.csv".to_string();
    route.resolved_intent = "skill=extension_manager action=assess_gap".to_string();
    route.route_reason = "capability=extension.assess_gap".to_string();

    assert!(
        directory_purpose_extension_inventory_deterministic_plan_result(
            "extension_manager assess_gap",
            Some(&route),
            &LoopState::new(1),
            Some(&root_path),
        )
        .is_none()
    );
}

#[test]
fn directory_purpose_reads_representative_found_files_after_extension_inventory() {
    use crate::executor::{StepExecutionResult, StepExecutionStatus};

    let root = TempDirGuard::new("directory_purpose_representative_reads");
    fs::create_dir_all(root.path.join("configs/channels")).expect("create config dirs");
    let cargo_path = root.path.join("Cargo.toml");
    let config_path = root.path.join("configs/config.toml");
    let channel_path = root.path.join("configs/channels/telegram.toml");
    fs::write(&cargo_path, "[workspace]\n").expect("write cargo");
    fs::write(&config_path, "[skills]\n").expect("write config");
    fs::write(&channel_path, "[telegram]\n").expect("write channel");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryPurposeSummary;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = "*.toml".to_string();
    let mut loop_state = LoopState::new(3);
    loop_state.round_no = 2;
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "find_ext",
                "count": 4,
                "ext": "toml",
                "results": [
                    "Cargo.toml",
                    "configs/config.toml",
                    "configs/channels/telegram.toml",
                    "missing.toml"
                ]
            })
            .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });

    let plan = directory_purpose_representative_reads_after_find_result(
        "summarize representative toml files",
        Some(&route),
        &loop_state,
        Some(&root_path),
    )
    .expect("representative read plan");

    assert_eq!(plan.plan_kind, PlanKind::Single);
    assert_eq!(plan.steps.len(), 5);
    let expected = [
        cargo_path.canonicalize().unwrap(),
        config_path.canonicalize().unwrap(),
        channel_path.canonicalize().unwrap(),
    ];
    for (idx, expected_path) in expected.iter().enumerate() {
        let action = plan.steps[idx].to_agent_action().expect("agent action");
        let args = expect_planned_call(&action, "fs_basic", "read_text_range");
        let expected_path = expected_path.display().to_string();
        assert_eq!(
            args.get("path").and_then(Value::as_str),
            Some(expected_path.as_str())
        );
        assert_eq!(args.get("mode").and_then(Value::as_str), Some("head"));
    }
    assert!(matches!(
        plan.steps.get(3).and_then(|step| step.to_agent_action()),
        Some(AgentAction::SynthesizeAnswer { evidence_refs })
            if evidence_refs == vec![
                "step_1".to_string(),
                "step_2".to_string(),
                "step_3".to_string()
            ]
    ));
    assert!(matches!(
        plan.steps.get(4).and_then(|step| step.to_agent_action()),
        Some(AgentAction::Respond { content }) if content == "{{last_output}}"
    ));
}

#[test]
fn directory_purpose_reads_representative_found_files_from_wrapped_extra() {
    use crate::executor::{StepExecutionResult, StepExecutionStatus};

    let root = TempDirGuard::new("directory_purpose_wrapped_representative_reads");
    let first_path = root.path.join("intent_normalizer.schema.json");
    let second_path = root.path.join("contract_repair_judge.schema.json");
    fs::write(&first_path, "{\"title\":\"intent\"}\n").expect("write first");
    fs::write(&second_path, "{\"title\":\"contract\"}\n").expect("write second");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryPurposeSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_path.clone();
    route.resolved_intent =
        "List .json files, find the largest schema, and summarize its purpose.".to_string();
    let mut loop_state = LoopState::new(3);
    loop_state.round_no = 2;
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            json!({
                "extra": {
                    "action": "find_ext",
                    "count": 2,
                    "ext": "json",
                    "results": [
                        "intent_normalizer.schema.json",
                        "contract_repair_judge.schema.json"
                    ]
                },
                "text": "{\"action\":\"find_ext\",\"count\":2}"
            })
            .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });

    let plan = directory_purpose_representative_reads_after_find_result(
        "summarize json schema directory",
        Some(&route),
        &loop_state,
        Some(&root_path),
    )
    .expect("representative read plan");

    assert_eq!(plan.plan_kind, PlanKind::Single);
    assert_eq!(plan.steps.len(), 4);
    let first = plan.steps[0].to_agent_action().expect("first action");
    let args = expect_planned_call(&first, "fs_basic", "read_text_range");
    assert!(args
        .get("path")
        .and_then(Value::as_str)
        .is_some_and(|path| path.ends_with("intent_normalizer.schema.json")));
    let second = plan.steps[1].to_agent_action().expect("second action");
    let args = expect_planned_call(&second, "fs_basic", "read_text_range");
    assert!(args
        .get("path")
        .and_then(Value::as_str)
        .is_some_and(|path| path.ends_with("contract_repair_judge.schema.json")));
    assert!(matches!(
        plan.steps.get(2).and_then(|step| step.to_agent_action()),
        Some(AgentAction::SynthesizeAnswer { evidence_refs })
            if evidence_refs == vec!["step_1".to_string(), "step_2".to_string()]
    ));
}

#[test]
fn directory_tree_auto_locator_does_not_override_exact_file_names_contract() {
    let root = TempDirGuard::new("directory_tree_auto_locator_file_names");
    fs::write(root.path.join("README.md"), "hello").expect("write readme");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::FileNames;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_path.clone();

    assert!(directory_tree_auto_locator_deterministic_plan_result(
        &test_state(),
        "list file names",
        Some(&route),
        &LoopState::new(1),
        "list file names",
        Some("list file names"),
        Some(&root_path),
    )
    .is_none());
}

#[test]
fn directory_tree_auto_locator_does_not_override_raw_command_output_contract() {
    let root = TempDirGuard::new("directory_tree_auto_locator_raw_command");
    fs::write(root.path.join("README.md"), "hello").expect("write readme");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_path.clone();

    assert!(directory_tree_auto_locator_deterministic_plan_result(
        &test_state(),
        "show current process output",
        Some(&route),
        &LoopState::new(1),
        "show current process output",
        Some("show current process output"),
        Some(&root_path),
    )
    .is_none());
}

#[test]
fn directory_tree_auto_locator_does_not_override_multi_directory_contract() {
    let root = TempDirGuard::new("directory_tree_auto_locator_multi_dir");
    fs::create_dir_all(root.path.join("left")).expect("create left");
    fs::create_dir_all(root.path.join("right")).expect("create right");
    let left_path = root.path.join("left").display().to_string();
    let right_path = root.path.join("right").display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = format!("{left_path} | {right_path}");

    assert!(directory_tree_auto_locator_deterministic_plan_result(
        &test_state(),
        "compare two directories",
        Some(&route),
        &LoopState::new(1),
        "compare two directories",
        Some("compare two directories"),
        Some(&left_path),
    )
    .is_none());
}

#[test]
fn scalar_path_respond_only_uses_auto_locator_observation() {
    let root = TempDirGuard::new("scalar_auto_locator_respond_only");
    let report = root.path.join("Report.MD");
    fs::write(&report, "hello").expect("write report");
    let report_path = report.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.delivery_required = false;
    let actions = vec![AgentAction::Respond {
        content: report_path.clone(),
    }];

    let normalized = replace_scalar_path_respond_only_with_auto_locator_observation(
        Some(&route),
        &LoopState::new(1),
        Some(&report_path),
        actions,
    );
    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
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
fn content_excerpt_summary_inserts_auto_locator_read_before_synthesis() {
    let root = TempDirGuard::new("content_excerpt_auto_locator");
    let readme = root.path.join("README.md");
    fs::write(&readme, "# RustClaw\n\nA local agent runtime.").expect("write readme");
    let readme_path = readme.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "path_batch_facts",
                "paths": ["definitely_missing_rustclaw_20260510.md"],
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

    let normalized = ensure_content_excerpt_summary_has_bounded_content(
        Some(&route),
        &loop_state,
        Some(&readme_path),
        actions,
    );

    assert_eq!(normalized.len(), 4);
    assert!(matches!(
        &normalized[1],
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "fs_basic"
                && args.get("action").and_then(Value::as_str) == Some("read_text_range")
                && args.get("path").and_then(Value::as_str) == Some(readme_path.as_str())
    ));
    assert!(matches!(
        &normalized[2],
        AgentAction::SynthesizeAnswer { evidence_refs } if evidence_refs == &vec!["last_output".to_string()]
    ));
}

#[test]
fn workspace_synthesis_respond_only_with_generic_semantic_uses_default_evidence() {
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.delivery_required = false;
    let actions = vec![AgentAction::Respond {
        content: "RustClaw 是一个本地智能助手平台。".to_string(),
    }];

    let normalized =
        replace_workspace_synthesis_respond_only_plan(Some(&route), &LoopState::new(1), actions);

    assert_eq!(normalized.len(), 6);
    assert!(matches!(
        &normalized[0],
        AgentAction::CallSkill { skill, args }
            if skill == "system_basic"
                && args.get("action").and_then(Value::as_str) == Some("workspace_glance")
    ));
    assert!(matches!(
        &normalized[1],
        AgentAction::CallTool { tool, args }
            if tool == "config_basic"
                && args.get("action").and_then(Value::as_str) == Some("read_fields")
    ));
    assert!(matches!(
        &normalized[3],
        AgentAction::CallTool { tool, args }
            if tool == "fs_basic"
                && args.get("action").and_then(Value::as_str) == Some("read_text_range")
    ));
    assert!(matches!(
    &normalized[4],
    AgentAction::SynthesizeAnswer { evidence_refs }
        if evidence_refs == &vec![
            "step_1".to_string(),
            "step_2".to_string(),
            "step_3".to_string(),
            "step_4".to_string(),
        ]
    ));
}

#[test]
fn workspace_default_evidence_requires_content_evidence_contract() {
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        false,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.delivery_required = false;
    let actions = vec![AgentAction::Respond {
        content: "plain chat answer".to_string(),
    }];

    let normalized =
        replace_workspace_synthesis_respond_only_plan(Some(&route), &LoopState::new(1), actions);

    assert_eq!(normalized.len(), 1);
    assert!(matches!(&normalized[0], AgentAction::Respond { .. }));
}

#[test]
fn workspace_summary_default_text_evidence_uses_contract_without_execute_gate() {
    let mut route = route_result(
        crate::AskMode::direct_answer(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::WorkspaceProjectSummary;
    route.output_contract.delivery_required = false;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({"action": "workspace_glance"}),
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = ensure_workspace_synthesis_has_default_text_evidence(
        Some(&route),
        &LoopState::new(1),
        actions,
    );

    assert_eq!(normalized.len(), 4);
    assert!(matches!(
        &normalized[1],
        AgentAction::CallSkill { skill, args }
            if skill == "git_basic"
                && args.get("action").and_then(Value::as_str) == Some("log")
    ));
    assert!(matches!(
        &normalized[2],
        AgentAction::CallTool { tool, args }
            if tool == "fs_basic"
                && args.get("action").and_then(Value::as_str) == Some("read_text_range")
                && args.get("path").and_then(Value::as_str) == Some("README.md")
    ));
}

#[test]
fn content_excerpt_summary_auto_locator_deterministic_plan_uses_doc_parse_for_loose_doc() {
    let root = TempDirGuard::new("content_excerpt_deterministic_plan");
    let readme = root.path.join("README.md");
    fs::write(&readme, "# RustClaw\n\nA local agent runtime.").expect("write readme");
    let readme_path = readme.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;
    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.join("workspace_root");

    let plan = content_excerpt_summary_auto_locator_deterministic_plan_result(
        &state,
        "summarize a resolved fallback document",
        Some(&route),
        &loop_state,
        Some(&readme_path),
    )
    .expect("content excerpt summary should parse the resolved document directly");

    assert_eq!(plan.plan_kind, PlanKind::Single);
    assert_eq!(plan.steps.len(), 3);
    match &plan.steps[0].to_agent_action() {
        Some(AgentAction::CallSkill { skill, args }) => {
            assert_eq!(skill, "doc_parse");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("parse_doc")
            );
            assert_eq!(
                args.get("path").and_then(Value::as_str),
                Some(readme_path.as_str())
            );
        }
        other => panic!("expected doc_parse parse_doc action, got {other:?}"),
    }
    assert!(matches!(
        plan.steps[1].to_agent_action(),
        Some(AgentAction::SynthesizeAnswer { evidence_refs }) if evidence_refs == vec!["last_output".to_string()]
    ));
    assert!(matches!(
        plan.steps[2].to_agent_action(),
        Some(AgentAction::Respond { content }) if content == "{{last_output}}"
    ));
}

#[test]
fn content_excerpt_summary_auto_locator_reads_nested_file_without_workspace_inventory() {
    let root = TempDirGuard::new("content_excerpt_workspace_context");
    let ui_dir = root.path.join("UI");
    fs::create_dir_all(&ui_dir).expect("create UI dir");
    let package_json = ui_dir.join("package.json");
    fs::write(&package_json, r#"{"name":"react-example","private":true}"#).expect("write package");
    fs::create_dir_all(root.path.join("crates")).expect("create crates dir");
    let package_path = package_json.display().to_string();
    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.delivery_required = false;
    route.output_contract.requires_content_evidence = true;
    route.resolved_intent = "Summarize package metadata. slice_mode=head slice_n=120".to_string();
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let plan = content_excerpt_summary_auto_locator_deterministic_plan_result(
        &state,
        "use workspace context and a resolved package file",
        Some(&route),
        &loop_state,
        Some(&package_path),
    )
    .expect("workspace file summary should include root context and file evidence");

    assert_eq!(plan.plan_kind, PlanKind::Single);
    assert_eq!(plan.steps.len(), 3);
    match &plan.steps[0].to_agent_action() {
        Some(AgentAction::CallTool { tool, args }) => {
            assert_eq!(tool, "fs_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("read_text_range")
            );
            assert_eq!(
                args.get("path").and_then(Value::as_str),
                Some(package_path.as_str())
            );
            assert_eq!(args.get("mode").and_then(Value::as_str), Some("head"));
            assert_eq!(args.get("n").and_then(Value::as_u64), Some(120));
        }
        other => panic!("expected fs_basic read_text_range action, got {other:?}"),
    }
    assert!(matches!(
        plan.steps[1].to_agent_action(),
        Some(AgentAction::SynthesizeAnswer { evidence_refs })
            if evidence_refs == vec!["last_output".to_string()]
    ));
    assert!(matches!(
        plan.steps[2].to_agent_action(),
        Some(AgentAction::Respond { content }) if content == "{{last_output}}"
    ));
}

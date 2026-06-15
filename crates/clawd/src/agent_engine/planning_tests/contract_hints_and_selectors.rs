use super::*;

#[test]
fn contract_hint_matrix_config_risk_uses_deterministic_guard_action() {
    let state = test_state_with_enabled_skills(&["config_basic", "config_edit"]);
    let mut route = base_route_result();
    route.route_reason = "structured_contract_hint_fast_path; contract_hint_fast_path".into();
    route.output_contract.requires_content_evidence = false;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = OutputSemanticKind::ConfigRiskAssessment;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();

    let plan = contract_hint_preferred_action_deterministic_plan_result(
        &state,
        "guard config",
        Some(&route),
        &LoopState::new(1),
        "sanitized request without hint block",
        None,
    )
    .expect("config risk contract should use deterministic guard action");

    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].skill, "config_basic");
    assert_eq!(
        plan.steps[0].args.get("action").and_then(Value::as_str),
        Some("guard_rustclaw_config")
    );
    assert_eq!(
        plan.steps[0].args.get("path").and_then(Value::as_str),
        Some("configs/config.toml")
    );
}

#[test]
fn contract_hint_preferred_config_guard_uses_runtime_equivalent_action() {
    let state = test_state_with_enabled_skills(&["config_basic", "config_edit"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = false;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = OutputSemanticKind::ConfigRiskAssessment;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();
    let request = "[CONTRACT_TEST_HINT]\npreferred_action_ref=config_guard\n[/CONTRACT_TEST_HINT]";

    let plan = contract_hint_preferred_action_deterministic_plan_result(
        &state,
        "guard config",
        Some(&route),
        &LoopState::new(1),
        request,
        None,
    )
    .expect("virtual config guard should map to runtime guard action");

    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].skill, "config_edit");
    assert_eq!(
        plan.steps[0].args.get("action").and_then(Value::as_str),
        Some("guard_config")
    );
}

#[test]
fn contract_hint_file_paths_uses_machine_selector_extension() {
    let state = test_state_with_enabled_skills(&["fs_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = OutputSemanticKind::FilePaths;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "scripts/nl_tests/fixtures/device_local".to_string();
    let request = "[CONTRACT_TEST_HINT]\nsemantic_kind=file_paths\nselector_extension=md\nselector_target_kind=file\n[/CONTRACT_TEST_HINT]";

    let plan = contract_hint_preferred_action_deterministic_plan_result(
        &state,
        "list markdown paths",
        Some(&route),
        &LoopState::new(1),
        request,
        None,
    )
    .expect("file path contract should use structured selector hints");

    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].skill, "fs_basic");
    assert_eq!(
        plan.steps[0].args.get("action").and_then(Value::as_str),
        Some("find_entries")
    );
    assert_eq!(
        plan.steps[0].args.get("root").and_then(Value::as_str),
        Some("scripts/nl_tests/fixtures/device_local")
    );
    assert_eq!(
        plan.steps[0].args.get("extension").and_then(Value::as_str),
        Some("md")
    );
    assert_eq!(
        plan.steps[0]
            .args
            .get("target_kind")
            .and_then(Value::as_str),
        Some("file")
    );
}

#[test]
fn contract_hint_recent_artifacts_uses_machine_sort_and_limit_selectors() {
    let state = test_state_with_enabled_skills(&["fs_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = OutputSemanticKind::RecentArtifactsJudgment;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "scripts/nl_tests/fixtures/device_local/docs".to_string();
    let request = "[CONTRACT_TEST_HINT]\nsemantic_kind=recent_artifacts_judgment\nselector_limit=2\nselector_sort_by=mtime_desc\nselector_target_kind=file\n[/CONTRACT_TEST_HINT]";

    let plan = contract_hint_preferred_action_deterministic_plan_result(
        &state,
        "list recent files and judge",
        Some(&route),
        &LoopState::new(1),
        request,
        None,
    )
    .expect("recent artifact contract should use structured sort selectors");

    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].skill, "fs_basic");
    assert_eq!(
        plan.steps[0].args.get("action").and_then(Value::as_str),
        Some("list_dir")
    );
    assert_eq!(
        plan.steps[0].args.get("sort_by").and_then(Value::as_str),
        Some("mtime_desc")
    );
    assert_eq!(
        plan.steps[0]
            .args
            .get("max_entries")
            .and_then(Value::as_u64),
        Some(2)
    );
    assert_eq!(
        plan.steps[0]
            .args
            .get("files_only")
            .and_then(Value::as_bool),
        Some(true)
    );
}

#[test]
fn contract_hint_file_names_uses_machine_file_kind_selector() {
    let state = test_state_with_enabled_skills(&["fs_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = OutputSemanticKind::FileNames;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "scripts/nl_tests/fixtures/device_local/docs".to_string();
    let request =
            "[CONTRACT_TEST_HINT]\nsemantic_kind=file_names\nselector_target_kind=file\n[/CONTRACT_TEST_HINT]";

    let plan = contract_hint_preferred_action_deterministic_plan_result(
        &state,
        "list file names",
        Some(&route),
        &LoopState::new(1),
        request,
        None,
    )
    .expect("file name contract should use file-only selector hints");

    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].skill, "fs_basic");
    assert_eq!(
        plan.steps[0].args.get("action").and_then(Value::as_str),
        Some("list_dir")
    );
    assert_eq!(
        plan.steps[0]
            .args
            .get("files_only")
            .and_then(Value::as_bool),
        Some(true)
    );
    assert!(
        plan.steps[0].args.get("dirs_only").is_none(),
        "file-only selector must not also request directories"
    );
}

#[test]
fn contract_hint_directory_entry_groups_find_entries_defaults_to_any_kind() {
    let state = test_state_with_enabled_skills(&["fs_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryEntryGroups;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "scripts/nl_tests/fixtures/device_local".to_string();
    let request = "[CONTRACT_TEST_HINT]\nsemantic_kind=directory_entry_groups\npreferred_action_ref=fs_basic.find_entries\n[/CONTRACT_TEST_HINT]";

    let plan = contract_hint_preferred_action_deterministic_plan_result(
        &state,
        "group direct children by kind",
        Some(&route),
        &LoopState::new(1),
        request,
        None,
    )
    .expect("directory entry grouping should preserve file and directory candidates");

    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].skill, "fs_basic");
    assert_eq!(
        plan.steps[0].args.get("action").and_then(Value::as_str),
        Some("find_entries")
    );
    assert_eq!(
        plan.steps[0]
            .args
            .get("target_kind")
            .and_then(Value::as_str),
        Some("any")
    );
}

#[test]
fn contract_hint_archive_read_uses_matrix_preferred_action_without_nl_matching() {
    let state = test_state_with_enabled_skills(&["archive_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = OutputSemanticKind::ArchiveRead;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip|notes.txt".to_string();
    let request = "[CONTRACT_TEST_HINT]\nsemantic_kind=archive_read\ncandidate_wrong_action_ref=fs_basic.find_entries\n[/CONTRACT_TEST_HINT]";

    let plan = contract_hint_preferred_action_deterministic_plan_result(
        &state,
        "read archive member",
        Some(&route),
        &LoopState::new(1),
        request,
        None,
    )
    .expect("archive read contract should use matrix preferred archive action");

    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].skill, "archive_basic");
    assert_eq!(
        plan.steps[0].args.get("action").and_then(Value::as_str),
        Some("read")
    );
    assert_eq!(
        plan.steps[0].args.get("archive").and_then(Value::as_str),
        Some("scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip")
    );
    assert_eq!(
        plan.steps[0].args.get("member").and_then(Value::as_str),
        Some("notes.txt")
    );
}

#[test]
fn contract_hint_content_presence_uses_machine_query_and_case_selector() {
    let state = test_state_with_enabled_skills(&["fs_basic"]);
    let mut route = base_route_result();
    route.route_reason = "structured_contract_hint_fast_path; contract_hint_fast_path".into();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentPresenceCheck;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md".to_string();
    let request = "[CONTRACT_TEST_HINT]\nsemantic_kind=content_presence_check\nselector_query=release\nselector_case_insensitive=true\n[/CONTRACT_TEST_HINT]";

    let plan = contract_hint_preferred_action_deterministic_plan_result(
        &state,
        "check content presence",
        Some(&route),
        &LoopState::new(1),
        request,
        None,
    )
    .expect("content presence contract should use structured query selector");

    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].skill, "fs_basic");
    assert_eq!(
        plan.steps[0].args.get("action").and_then(Value::as_str),
        Some("grep_text")
    );
    assert_eq!(
        plan.steps[0].args.get("query").and_then(Value::as_str),
        Some("release")
    );
    assert_eq!(
        plan.steps[0]
            .args
            .get("case_insensitive")
            .and_then(Value::as_bool),
        Some(true)
    );
}

#[test]
fn contract_hint_preferred_doc_parse_uses_structured_parse_doc_action() {
    let state = test_state_with_enabled_skills(&["doc_parse"]);
    let mut route = base_route_result();
    route.route_reason = "structured_contract_hint_fast_path; contract_hint_fast_path".into();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentPresenceCheck;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md".to_string();
    let request = "[CONTRACT_TEST_HINT]\nsemantic_kind=content_presence_check\npreferred_action_ref=doc_parse\nselector_query=release\nselector_case_insensitive=true\n[/CONTRACT_TEST_HINT]";

    let plan = contract_hint_preferred_action_deterministic_plan_result(
        &state,
        "check content presence using preferred parser",
        Some(&route),
        &LoopState::new(1),
        request,
        None,
    )
    .expect("doc_parse preference should be planned without model fallback");

    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].skill, "doc_parse");
    assert_eq!(
        plan.steps[0].args.get("action").and_then(Value::as_str),
        Some("parse_doc")
    );
    assert_eq!(
        plan.steps[0].args.get("path").and_then(Value::as_str),
        Some("scripts/nl_tests/fixtures/device_local/docs/release_checklist.md")
    );
}

#[test]
fn quoted_literal_content_presence_uses_deterministic_grep_plan() {
    let root = TempDirGuard::new("quoted_literal_content_presence");
    let target = root.path.join("virtual_tools.rs");
    fs::write(&target, "pub const MARKER: &str = \"NEEDLE_TOKEN_123\";\n").expect("write target");
    let target_path = target.display().to_string();
    let mut state = test_state_with_enabled_skills(&["fs_basic"]);
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ContentPresenceCheck;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = target_path.clone();
    route.resolved_intent =
        "Check virtual_tools.rs for the quoted marker NEEDLE_TOKEN_123.".to_string();
    let request = "Check virtual_tools.rs for “NEEDLE_TOKEN_123”.";

    let plan = super::super::content_presence_quoted_literal_deterministic_plan_result(
        &state,
        request,
        Some(&route),
        &LoopState::new(1),
        request,
        Some(request),
        Some(target_path.as_str()),
    )
    .expect("quoted literal content presence should use grep_text");

    assert_eq!(plan.steps.len(), 1);
    let first = plan.steps[0]
        .to_agent_action()
        .expect("first step should be an action");
    let args = expect_planned_call(&first, "fs_basic", "grep_text");
    assert_eq!(
        args.get("root").and_then(Value::as_str),
        Some(target_path.as_str())
    );
    assert_eq!(
        args.get("query").and_then(Value::as_str),
        Some("NEEDLE_TOKEN_123")
    );
}

#[test]
fn contract_hint_hidden_entries_list_dir_includes_hidden_entries() {
    let state = test_state_with_enabled_skills(&["fs_basic"]);
    let mut route = base_route_result();
    route.route_reason = "structured_contract_hint_fast_path; contract_hint_fast_path".into();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = OutputSemanticKind::HiddenEntriesCheck;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = ".".to_string();
    let request = "[CONTRACT_TEST_HINT]\nsemantic_kind=hidden_entries_check\npreferred_action_ref=fs_basic.list_dir\n[/CONTRACT_TEST_HINT]";

    let plan = contract_hint_preferred_action_deterministic_plan_result(
        &state,
        "check hidden entries",
        Some(&route),
        &LoopState::new(1),
        request,
        None,
    )
    .expect("hidden entries contract should use deterministic inventory");

    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].skill, "fs_basic");
    assert_eq!(
        plan.steps[0].args.get("action").and_then(Value::as_str),
        Some("list_dir")
    );
    assert_eq!(
        plan.steps[0]
            .args
            .get("include_hidden")
            .and_then(Value::as_bool),
        Some(true)
    );
}

#[test]
fn directory_entry_groups_selector_include_hidden_reaches_list_dir_args() {
    let root = TempDirGuard::new("directory_entry_groups_include_hidden");
    fs::write(root.path.join(".env"), "hidden").expect("write hidden file");
    fs::write(root.path.join("visible.txt"), "visible").expect("write visible file");
    let root_path = root.path.display().to_string();
    let mut state = test_state_with_enabled_skills(&["fs_basic"]);
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryEntryGroups;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = root_path.clone();
    route.output_contract.delivery_required = false;
    route
        .output_contract
        .self_extension
        .list_selector
        .include_hidden = Some(true);
    route.output_contract.self_extension.list_selector.limit = Some(3);

    let plan = directory_entry_groups_auto_locator_deterministic_plan_result(
        &state,
        "inspect current workspace entries with hidden entries included",
        Some(&route),
        &LoopState::new(1),
        "inspect current workspace entries",
        None,
        Some(root_path.as_str()),
    )
    .expect("directory entry groups plan should use deterministic inventory");

    assert_eq!(plan.steps.len(), 1);
    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "fs_basic", "list_dir");
    assert_eq!(
        args.get("include_hidden").and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(args.get("max_entries").and_then(Value::as_u64), Some(3));
}

#[test]
fn fs_basic_grep_text_case_sensitive_false_normalizes_to_case_insensitive() {
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "grep_text",
            "path": "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md",
            "query": "release",
            "case_sensitive": false,
            "max_matches": 3
        }),
    }];

    let normalized = normalize_fs_basic_schema_aliases(actions);
    let args = expect_planned_call(&normalized[0], "fs_basic", "grep_text");
    assert_eq!(
        args.get("case_insensitive").and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(args.get("max_results").and_then(Value::as_u64), Some(3));
}

#[test]
fn fs_basic_read_text_range_range_tail_alias_becomes_mode_tail() {
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "read_text_range",
            "path": "logs/model_io.log",
            "range": "tail",
            "n": 4
        }),
    }];

    let normalized = normalize_fs_basic_schema_aliases(actions);
    let args = expect_planned_call(&normalized[0], "fs_basic", "read_text_range");
    assert_eq!(args.get("mode").and_then(Value::as_str), Some("tail"));
    assert_eq!(args.get("n").and_then(Value::as_u64), Some(4));
    assert!(args.get("range").is_none());
}

#[test]
fn fs_basic_read_text_range_negative_start_line_count_becomes_tail_count() {
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "read_text_range",
            "path": "logs/model_io.log",
            "start_line": -4,
            "line_count": 4
        }),
    }];

    let normalized = normalize_fs_basic_schema_aliases(actions);
    let args = expect_planned_call(&normalized[0], "fs_basic", "read_text_range");
    assert_eq!(args.get("mode").and_then(Value::as_str), Some("tail"));
    assert_eq!(args.get("n").and_then(Value::as_u64), Some(4));
    assert!(args.get("start_line").is_none());
    assert!(args.get("line_count").is_none());
}

#[test]
fn service_status_process_request_uses_process_basic_filter_plan() {
    let state = test_state_with_enabled_skills(&["process_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    let loop_state = LoopState::new(1);

    let plan = service_status_deterministic_plan_result(
        &state,
        "check clawd process",
        Some(&route),
        &loop_state,
        "check whether the local clawd process is present",
    )
    .expect("process status should use deterministic process_basic plan");

    assert_eq!(plan.steps.len(), 1);
    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "process_basic", "ps");
    assert_eq!(args.get("filter").and_then(Value::as_str), Some("clawd"));
    assert_eq!(args.get("limit").and_then(Value::as_u64), Some(200));
}

#[test]
fn service_status_url_request_uses_http_basic_plan() {
    let state = test_state_with_enabled_skills(&["process_basic", "http_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    route.resolved_intent = "访问 http://127.0.0.1:8787/v1/health，简短告诉我结果".to_string();
    let loop_state = LoopState::new(1);

    let plan = service_status_deterministic_plan_result(
        &state,
        "observe local health URL",
        Some(&route),
        &loop_state,
        "访问 http://127.0.0.1:8787/v1/health，简短告诉我结果",
    )
    .expect("URL status request should use http_basic");

    assert_eq!(plan.steps.len(), 3);
    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "http_basic", "get");
    assert_eq!(
        args.get("url").and_then(Value::as_str),
        Some("http://127.0.0.1:8787/v1/health")
    );
}

#[test]
fn service_status_workspace_product_request_uses_health_check_plan() {
    let mut state = test_state_with_enabled_skills(&["health_check", "process_basic"]);
    let tmp = TempDirGuard::new("rustclaw");
    let project_root = tmp.path.join("rustclaw");
    fs::create_dir_all(&project_root).expect("project root");
    state.skill_rt.workspace_root = project_root;
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    let loop_state = LoopState::new(1);

    let plan = service_status_deterministic_plan_result(
        &state,
        "check local project service health",
        Some(&route),
        &loop_state,
        "检查本地 RustClaw 服务健康状态，简短输出状态",
    )
    .expect("workspace product status should use health_check plan");

    assert_eq!(plan.steps.len(), 1);
    let action = plan.steps[0].to_agent_action().expect("agent action");
    match action {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "health_check");
            assert!(args.as_object().is_some_and(|obj| obj.is_empty()));
        }
        other => panic!("expected health_check action, got {other:?}"),
    }
}

#[test]
fn service_status_health_check_recipe_marker_uses_health_check_plan() {
    let state = test_state_with_enabled_skills(&["health_check", "process_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.route_reason = "execution_recipe_health_check_observation".to_string();
    let loop_state = LoopState::new(1);

    let plan = service_status_deterministic_plan_result(
        &state,
        "run a structured health observation",
        Some(&route),
        &loop_state,
        "run a structured health observation",
    )
    .expect("health check recipe marker should use health_check");

    assert_eq!(plan.steps.len(), 1);
    let action = plan.steps[0].to_agent_action().expect("agent action");
    match action {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "health_check");
            assert!(args.as_object().is_some_and(|obj| obj.is_empty()));
        }
        other => panic!("expected health_check action, got {other:?}"),
    }
}

#[test]
fn scalar_service_status_uses_health_check_plan() {
    let state = test_state_with_enabled_skills(&["health_check", "process_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    let loop_state = LoopState::new(1);

    let plan = service_status_deterministic_plan_result(
        &state,
        "return one runtime scalar",
        Some(&route),
        &loop_state,
        "current runtime scalar",
    )
    .expect("scalar service status should use health check");

    assert_eq!(plan.steps.len(), 1);
    let action = plan.steps[0].to_agent_action().expect("agent action");
    match action {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "health_check");
            assert!(args.as_object().is_some_and(|obj| obj.is_empty()));
        }
        other => panic!("expected health_check action, got {other:?}"),
    }
}

#[test]
fn scalar_service_status_named_process_uses_process_basic_filter_plan() {
    let state = test_state_with_enabled_skills(&["health_check", "process_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    let loop_state = LoopState::new(1);

    let plan = service_status_deterministic_plan_result(
        &state,
        "check named service",
        Some(&route),
        &loop_state,
        "telegramd",
    )
    .expect("named service status should use process_basic");

    assert_eq!(plan.steps.len(), 1);
    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "process_basic", "ps");
    assert_eq!(
        args.get("filter").and_then(Value::as_str),
        Some("telegramd")
    );
    assert_eq!(args.get("limit").and_then(Value::as_u64), Some(200));
}

#[test]
fn structural_contracts_are_not_blocked_by_literal_command_guard() {
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::StructuredKeys;
    route.output_contract.locator_hint = "package.json".to_string();

    assert!(structural_contract_deterministic_plan_overrides_literal_command_guard(Some(&route)));
}

#[test]
fn service_status_port_request_uses_process_basic_port_filter_plan() {
    let state = test_state_with_enabled_skills(&["process_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    let loop_state = LoopState::new(1);

    let plan = service_status_deterministic_plan_result(
        &state,
        "check local port",
        Some(&route),
        &loop_state,
        "show the process listening on local port 8787",
    )
    .expect("port status should use deterministic process_basic plan");

    assert_eq!(plan.steps.len(), 1);
    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "process_basic", "port_list");
    assert_eq!(args.get("filter").and_then(Value::as_str), Some("8787"));
}

#[test]
fn service_status_process_ranking_count_is_not_port_filter() {
    let state = test_state_with_enabled_skills(&["process_basic", "system_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    let loop_state = LoopState::new(1);

    let plan = service_status_deterministic_plan_result(
        &state,
        "observe process ranking",
        Some(&route),
        &loop_state,
        "看一下当前最占 CPU 的前 5 个进程，简短告诉我最值得注意的是哪个",
    );

    if let Some(plan) = plan {
        for step in plan.steps {
            if let Some(action) = step.to_agent_action() {
                assert!(
                    !planned_call_is(&action, "process_basic", "port_list"),
                    "process ranking count must not be treated as a port filter: {action:?}"
                );
            }
        }
    }
}

#[test]
fn service_status_without_process_target_uses_system_basic_info_plan() {
    let state = test_state_with_enabled_skills(&["system_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    let loop_state = LoopState::new(1);

    let plan = service_status_deterministic_plan_result(
        &state,
        "observe local runtime identity",
        Some(&route),
        &loop_state,
        "observe local runtime identity",
    )
    .expect("system status fallback should use system_basic info");

    assert_eq!(plan.steps.len(), 1);
    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "system_basic", "info");
    assert_eq!(args.as_object().map(|obj| obj.len()), Some(1));
}

#[test]
fn service_status_identity_field_prefers_system_basic_info_over_health_check() {
    let state = test_state_with_enabled_skills(&["health_check", "system_basic"]);
    let mut route = base_route_result();
    route.resolved_intent = "Show current hostname and current user.".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    let loop_state = LoopState::new(1);

    let plan = service_status_deterministic_plan_result(
        &state,
        "observe local runtime identity",
        Some(&route),
        &loop_state,
        "show hostname and current_user",
    )
    .expect("identity field request should use system_basic info");

    assert_eq!(plan.steps.len(), 1);
    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "system_basic", "info");
    assert_eq!(args.as_object().map(|obj| obj.len()), Some(1));
}

#[test]
fn service_status_generic_status_prefers_health_check_over_system_basic_info() {
    let state = test_state_with_enabled_skills(&["health_check", "process_basic", "system_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    let loop_state = LoopState::new(1);

    let plan = service_status_deterministic_plan_result(
        &state,
        "observe local runtime identity",
        Some(&route),
        &loop_state,
        "observe local runtime identity",
    )
    .expect("generic system status should use health_check");

    assert_eq!(plan.steps.len(), 1);
    let action = plan.steps[0].to_agent_action().expect("agent action");
    let AgentAction::CallSkill { skill, args } = action else {
        panic!("expected health_check call, got {action:?}");
    };
    assert_eq!(skill, "health_check");
    assert_eq!(args.as_object().map(|obj| obj.len()), Some(0));
}

#[test]
fn package_manager_dry_run_uses_commandish_answer_candidate() {
    let state = test_state_with_enabled_skills(&["package_manager"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.resolved_intent =
        "Show package preview\nanswer_candidate: command: sudo -n apt-get install -y ripgrep"
            .to_string();
    let loop_state = LoopState::new(1);

    let plan = package_manager_dry_run_deterministic_plan_result(
        &state,
        "dry-run package install",
        Some(&route),
        &loop_state,
        "ripgrep 설치는 하지 말고 dry-run 으로 어떤 명령이 될지만 알려줘.",
    )
    .expect("package manager dry-run should use deterministic plan");

    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "package_manager", "smart_install");
    assert_eq!(
        args.get("packages")
            .and_then(Value::as_array)
            .map(|packages| {
                packages
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
            }),
        Some(vec!["ripgrep"])
    );
    assert_eq!(args.get("dry_run").and_then(Value::as_bool), Some(true));
}

#[test]
fn package_manager_dry_run_falls_back_to_current_request_package_token() {
    let state = test_state_with_enabled_skills(&["package_manager"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.resolved_intent =
        "ripgrep install dry-run preview without executing installation".to_string();
    let loop_state = LoopState::new(1);

    let plan = package_manager_dry_run_deterministic_plan_result(
        &state,
        "dry-run package install",
        Some(&route),
        &loop_state,
        "ripgrep 설치는 하지 말고 dry-run 으로 어떤 명령이 될지만 알려줘.",
    )
    .expect("package manager dry-run should extract the safe current-request package token");

    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "package_manager", "smart_install");
    assert_eq!(
        args.get("packages")
            .and_then(Value::as_array)
            .map(|packages| {
                packages
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
            }),
        Some(vec!["ripgrep"])
    );
}

#[test]
fn archive_basic_unknown_readonly_action_normalizes_to_list_for_archive_contract() {
    let archive = "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip";
    let mut route = base_route_result();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_hint = archive.to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "archive_basic".to_string(),
        args: json!({
            "action": "exists",
            "archive": archive,
            "entry": "nested/config.ini",
        }),
    }];

    let normalized = normalize_archive_basic_schema_aliases(Some(&route), actions);

    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "archive_basic");
            assert_eq!(args.get("action").and_then(Value::as_str), Some("list"));
            assert_eq!(args.get("archive").and_then(Value::as_str), Some(archive));
        }
        other => panic!("expected archive_basic action, got {other:?}"),
    }
}

#[test]
fn archive_basic_unknown_mutating_shape_does_not_normalize_to_list() {
    let archive = "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip";
    let mut route = base_route_result();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_hint = archive.to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "archive_basic".to_string(),
        args: json!({
            "action": "make_archive",
            "source": "scripts/nl_tests/fixtures/device_local/docs",
            "archive": archive,
        }),
    }];

    let normalized = normalize_archive_basic_schema_aliases(Some(&route), actions);

    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "archive_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("make_archive")
            );
        }
        other => panic!("expected archive_basic action, got {other:?}"),
    }
}

#[test]
fn preferred_route_allows_more_specific_structured_tool_action() {
    let state = test_state_with_registry();
    let loop_state = LoopState::new(2);
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_hint = "tmp/nl_archive_case.zip".to_string();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "archive_basic".to_string(),
            args: json!({
                "action": "pack",
                "source": "scripts/skill_calls",
                "archive": "tmp/nl_archive_case.zip",
                "format": "zip"
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{s2.text}}".to_string(),
        },
    ];

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
fn process_ps_run_cmd_rewrites_to_process_basic() {
    let state = test_state_with_enabled_skills(&["process_basic", "run_cmd"]);
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": "ps -eo pid,user,%cpu,cmd --sort=-%cpu | head -n 6"}),
    }];

    let rewritten = rewrite_process_ps_run_cmd_to_process_basic(
        &state,
        "看一下当前最占 CPU 的前 5 个进程",
        None,
        actions,
    );

    let args = expect_planned_call(&rewritten[0], "process_basic", "ps");
    assert_eq!(args.get("limit").and_then(Value::as_u64), Some(5));
}

#[test]
fn process_ps_run_cmd_preserves_explicit_literal_command() {
    let state = test_state_with_enabled_skills(&["process_basic", "run_cmd"]);
    let command = "ps -eo pid,user,%cpu,cmd --sort=-%cpu | head -n 6";
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": command}),
    }];

    let rewritten = rewrite_process_ps_run_cmd_to_process_basic(
        &state,
        &format!("执行 {command}"),
        None,
        actions,
    );

    match &rewritten[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "run_cmd");
            assert_eq!(args.get("command").and_then(Value::as_str), Some(command));
        }
        other => panic!("expected run_cmd action, got {other:?}"),
    }
}

#[test]
fn process_ps_run_cmd_preserves_literal_flag() {
    let state = test_state_with_enabled_skills(&["process_basic", "run_cmd"]);
    let command = "ps -eo pid,user,%cpu,cmd --sort=-%cpu | head -n 6";
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({
            "command": command,
            CLAWD_LITERAL_COMMAND_ARG: true,
        }),
    }];

    let rewritten = rewrite_process_ps_run_cmd_to_process_basic(
        &state,
        "看一下当前最占 CPU 的前 5 个进程",
        None,
        actions,
    );

    match &rewritten[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "run_cmd");
            assert_eq!(args.get("command").and_then(Value::as_str), Some(command));
        }
        other => panic!("expected run_cmd action, got {other:?}"),
    }
}

#[test]
fn docker_ps_run_cmd_rewrites_to_docker_basic() {
    let state = test_state_with_enabled_skills(&["docker_basic", "run_cmd"]);
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": "docker ps -a"}),
    }];

    let rewritten = rewrite_docker_readonly_run_cmd_to_docker_basic(&state, false, actions);

    match &rewritten[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "docker_basic");
            assert_eq!(args.get("action").and_then(Value::as_str), Some("ps"));
        }
        other => panic!("expected docker_basic action, got {other:?}"),
    }
}

#[test]
fn docker_image_ls_run_cmd_rewrites_to_docker_basic_images() {
    let state = test_state_with_enabled_skills(&["docker_basic", "run_cmd"]);
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": "docker image ls"}),
    }];

    let rewritten = rewrite_docker_readonly_run_cmd_to_docker_basic(&state, false, actions);

    match &rewritten[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "docker_basic");
            assert_eq!(args.get("action").and_then(Value::as_str), Some("images"));
        }
        other => panic!("expected docker_basic action, got {other:?}"),
    }
}

#[test]
fn docker_version_run_cmd_rewrites_to_docker_basic_version() {
    let state = test_state_with_enabled_skills(&["docker_basic", "run_cmd"]);
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": "docker version"}),
    }];

    let rewritten = rewrite_docker_readonly_run_cmd_to_docker_basic(&state, false, actions);

    match &rewritten[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "docker_basic");
            assert_eq!(args.get("action").and_then(Value::as_str), Some("version"));
        }
        other => panic!("expected docker_basic action, got {other:?}"),
    }
}

#[test]
fn docker_readonly_preserves_explicit_literal_run_cmd() {
    let state = test_state_with_enabled_skills(&["docker_basic", "run_cmd"]);
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": "docker ps"}),
    }];

    let rewritten = rewrite_docker_readonly_run_cmd_to_docker_basic(&state, true, actions);

    match &rewritten[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "run_cmd");
            assert_eq!(
                args.get("command").and_then(Value::as_str),
                Some("docker ps")
            );
        }
        other => panic!("expected run_cmd action, got {other:?}"),
    }
}

#[test]
fn doc_parse_unsupported_transform_action_normalizes_to_parse_doc() {
    let state = test_state_with_enabled_skills(&["doc_parse"]);
    let actions = vec![AgentAction::CallSkill {
        skill: "doc_parse".to_string(),
        args: json!({
            "action": "summarize",
            "file_path": "/home/guagua/rustclaw/README.md",
            "max_chars": 8000
        }),
    }];

    let normalized = normalize_planned_actions(
        &state,
        Some(&base_route_result()),
        &LoopState::default(),
        "Summarize README.md",
        None,
        actions,
    );

    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "doc_parse");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("parse_doc")
            );
            assert_eq!(
                args.get("path").and_then(Value::as_str),
                Some("/home/guagua/rustclaw/README.md")
            );
        }
        other => panic!("expected doc_parse action, got {other:?}"),
    }
}

#[test]
fn archive_auto_locator_plans_list_instead_of_text_read() {
    let state = test_state_with_enabled_skills(&["archive_basic"]);
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.resolved_intent = "Inspect the archive contents without unpacking it.".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip".to_string();
    let loop_state = LoopState::new(1);

    assert!(
        scalar_content_auto_locator_observation_plan(
            Some(&route),
            Some("scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip"),
        )
        .is_none(),
        "archive files must not be planned as text reads"
    );

    let plan = archive_list_auto_locator_deterministic_plan_result(
        "Inspect the archive",
        &state,
        Some(&route),
        &loop_state,
        Some("scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip"),
    )
    .expect("archive list plan");

    assert_eq!(plan.steps.len(), 3);
    let step = &plan.steps[0];
    assert_eq!(step.action_type, "call_skill");
    assert_eq!(step.skill, "archive_basic");
    assert_eq!(
        step.args.get("action").and_then(Value::as_str),
        Some("list")
    );
    assert_eq!(
        step.args.get("archive").and_then(Value::as_str),
        Some("scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip")
    );
}

#[test]
fn archive_read_contract_plans_direct_member_read() {
    let state = test_state_with_enabled_skills(&["archive_basic"]);
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.resolved_intent =
        "Read member notes.txt from scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip"
            .to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::ArchiveRead;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip | notes.txt".to_string();
    let loop_state = LoopState::new(1);

    let plan = archive_read_deterministic_plan_result(
        "read archive member",
        &state,
        Some(&route),
        &loop_state,
        Some("scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip"),
        "Read member notes.txt from scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip",
    )
    .expect("archive read plan");

    assert_eq!(plan.steps.len(), 1);
    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "archive_basic", "read");
    assert_eq!(
        args.get("archive").and_then(Value::as_str),
        Some("scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip")
    );
    assert_eq!(
        args.get("member").and_then(Value::as_str),
        Some("notes.txt")
    );
}

#[test]
fn archive_read_contract_ignores_non_archive_auto_locator() {
    let state = test_state_with_enabled_skills(&["archive_basic"]);
    let archive = "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip";
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.resolved_intent = format!("Read notes.txt from {archive}");
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::ArchiveRead;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.locator_hint = format!("{archive} | notes.txt");
    let loop_state = LoopState::new(1);

    let plan = archive_read_deterministic_plan_result(
        "read archive member",
        &state,
        Some(&route),
        &loop_state,
        Some("/home/guagua/rustclaw/tmp/contract_matrix_unpacked/notes.txt"),
        &format!("Read member notes.txt from {archive}"),
    )
    .expect("archive read plan should fall back to contract locator");

    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "archive_basic", "read");
    assert_eq!(args.get("archive").and_then(Value::as_str), Some(archive));
    assert_eq!(
        args.get("member").and_then(Value::as_str),
        Some("notes.txt")
    );
}

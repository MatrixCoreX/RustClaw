use super::*;

#[test]
fn config_risk_capability_ref_allows_planner_supplied_guard_action() {
    let state = test_state_with_enabled_skills(&["config_basic", "config_edit"]);
    let mut route = base_route_result();
    route.route_reason = "capability_ref=config.guard_rustclaw_config".into();
    route.output_contract.requires_content_evidence = false;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();

    let args = assert_planner_supplied_skill_call_preserved(
        &state,
        &route,
        &LoopState::new(1),
        "guard config",
        Some("sanitized request without hint block"),
        Some(&route.route_reason),
        "config_basic",
        "guard_rustclaw_config",
        json!({"action": "guard_rustclaw_config", "path": "configs/config.toml"}),
    );
    assert_eq!(
        args.get("action").and_then(Value::as_str),
        Some("guard_rustclaw_config")
    );
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some("configs/config.toml")
    );
}

#[test]
fn config_guard_after_change_allows_planner_supplied_runtime_equivalent_action() {
    let state = test_state_with_enabled_skills(&["config_basic", "config_edit"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = false;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = OutputSemanticKind::ConfigRiskAssessment;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();
    route.route_reason = "capability_ref=config.guard_after_change".to_string();

    let action = AgentAction::CallSkill {
        skill: "config_edit".to_string(),
        args: json!({"action": "guard_config", "path": "configs/config.toml"}),
    };
    let AgentAction::CallSkill { skill, args } = &action else {
        unreachable!("test action is a skill call");
    };
    assert!(
        crate::evidence_policy::capability_ref_action_policy_for_route(Some(&route), skill, args)
            .is_some_and(|policy| policy.is_allowed())
    );

    let normalized = normalize_planned_actions_with_original_and_context(
        &state,
        Some(&route),
        &LoopState::new(1),
        "guard config",
        Some("[CONTRACT_TEST_HINT]\npreferred_action_ref=config_guard\n[/CONTRACT_TEST_HINT]"),
        Some(&route.route_reason),
        None,
        vec![action],
    );
    let args = normalized
        .iter()
        .find_map(|action| {
            planned_call_is(action, "config_basic", "guard_rustclaw_config")
                .then(|| expect_planned_call(action, "config_basic", "guard_rustclaw_config"))
        })
        .expect("config_edit guard_config should normalize to config_basic guard_rustclaw_config");

    assert_eq!(
        args.get("action").and_then(Value::as_str),
        Some("guard_rustclaw_config")
    );
}

#[test]
fn filesystem_find_entries_preserves_planner_supplied_extension_filter() {
    let state = test_state_with_enabled_skills(&["fs_basic"]);
    let mut route = base_route_result();
    route.route_reason = "capability_ref=filesystem.find_entries".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = OutputSemanticKind::FilePaths;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "scripts/nl_tests/fixtures/device_local".to_string();

    let args = assert_planner_supplied_skill_call_preserved(
        &state,
        &route,
        &LoopState::new(1),
        "list markdown paths",
        Some(
            "[CONTRACT_TEST_HINT]\nsemantic_kind=file_paths\nselector_extension=md\nselector_target_kind=file\n[/CONTRACT_TEST_HINT]",
        ),
        Some(&route.route_reason),
        "fs_basic",
        "find_entries",
        json!({
            "action": "find_entries",
            "root": "scripts/nl_tests/fixtures/device_local",
            "ext": "md",
            "target_kind": "file"
        }),
    );

    assert_eq!(
        args.get("action").and_then(Value::as_str),
        Some("find_entries")
    );
    assert_eq!(
        args.get("root").and_then(Value::as_str),
        Some("scripts/nl_tests/fixtures/device_local")
    );
    assert_eq!(args.get("ext").and_then(Value::as_str), Some("md"));
    assert_eq!(
        args.get("target_kind").and_then(Value::as_str),
        Some("file")
    );
}

#[test]
fn recent_artifacts_preserves_planner_supplied_sort_and_limit() {
    let state = test_state_with_enabled_skills(&["fs_basic"]);
    let mut route = base_route_result();
    route.route_reason = "capability_ref=filesystem.list_dir selector_target_kind=file".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = OutputSemanticKind::RecentArtifactsJudgment;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "scripts/nl_tests/fixtures/device_local/docs".to_string();

    let args = assert_planner_supplied_skill_call_preserved(
        &state,
        &route,
        &LoopState::new(1),
        "list recent files and judge",
        Some(
            "[CONTRACT_TEST_HINT]\nsemantic_kind=recent_artifacts_judgment\nselector_limit=2\nselector_sort_by=mtime_desc\nselector_target_kind=file\n[/CONTRACT_TEST_HINT]",
        ),
        Some(&route.route_reason),
        "fs_basic",
        "list_dir",
        json!({
            "action": "list_dir",
            "path": "scripts/nl_tests/fixtures/device_local/docs",
            "sort_by": "mtime_desc",
            "max_entries": 2,
            "files_only": true
        }),
    );

    assert_eq!(args.get("action").and_then(Value::as_str), Some("list_dir"));
    assert_eq!(
        args.get("sort_by").and_then(Value::as_str),
        Some("mtime_desc")
    );
    assert_eq!(args.get("max_entries").and_then(Value::as_u64), Some(2));
    assert_eq!(args.get("files_only").and_then(Value::as_bool), Some(true));
    assert_eq!(args.get("dirs_only").and_then(Value::as_bool), Some(false));
}

#[test]
fn file_names_preserves_planner_supplied_file_only_listing() {
    let state = test_state_with_enabled_skills(&["fs_basic"]);
    let mut route = base_route_result();
    route.route_reason = "capability_ref=filesystem.list_dir".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = OutputSemanticKind::FileNames;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "scripts/nl_tests/fixtures/device_local/docs".to_string();

    let args = assert_planner_supplied_skill_call_preserved(
        &state,
        &route,
        &LoopState::new(1),
        "list file names",
        Some(
            "[CONTRACT_TEST_HINT]\nsemantic_kind=file_names\nselector_target_kind=file\n[/CONTRACT_TEST_HINT]",
        ),
        Some(&route.route_reason),
        "fs_basic",
        "list_dir",
        json!({
            "action": "list_dir",
            "path": "scripts/nl_tests/fixtures/device_local/docs",
            "files_only": true
        }),
    );

    assert_eq!(args.get("action").and_then(Value::as_str), Some("list_dir"));
    assert_eq!(args.get("files_only").and_then(Value::as_bool), Some(true));
    assert_eq!(args.get("dirs_only").and_then(Value::as_bool), Some(false));
}

#[test]
fn directory_entry_groups_preserves_planner_supplied_any_kind_find_entries() {
    let state = test_state_with_enabled_skills(&["fs_basic"]);
    let mut route = base_route_result();
    route.route_reason = "capability_ref=filesystem.find_entries".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryEntryGroups;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "scripts/nl_tests/fixtures/device_local".to_string();

    let args = assert_planner_supplied_skill_call_preserved(
        &state,
        &route,
        &LoopState::new(1),
        "group direct children by kind",
        Some(
            "[CONTRACT_TEST_HINT]\nsemantic_kind=directory_entry_groups\npreferred_action_ref=fs_basic.find_entries\n[/CONTRACT_TEST_HINT]",
        ),
        Some(&route.route_reason),
        "fs_basic",
        "find_entries",
        json!({
            "action": "find_entries",
            "root": "scripts/nl_tests/fixtures/device_local",
            "target_kind": "any"
        }),
    );

    assert_eq!(
        args.get("action").and_then(Value::as_str),
        Some("find_entries")
    );
    assert_eq!(args.get("target_kind").and_then(Value::as_str), Some("any"));
}

#[test]
fn archive_read_preserves_planner_supplied_member_read() {
    let state = test_state_with_enabled_skills(&["archive_basic"]);
    let mut route = base_route_result();
    route.route_reason = "capability_ref=archive.read".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip|notes.txt".to_string();

    let args = assert_planner_supplied_skill_call_preserved(
        &state,
        &route,
        &LoopState::new(1),
        "read archive member",
        Some(
            "[CONTRACT_TEST_HINT]\ncandidate_wrong_action_ref=fs_basic.find_entries\n[/CONTRACT_TEST_HINT]",
        ),
        Some(&route.route_reason),
        "archive_basic",
        "read",
        json!({
            "action": "read",
            "archive": "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip",
            "member": "notes.txt"
        }),
    );

    assert_eq!(args.get("action").and_then(Value::as_str), Some("read"));
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
fn content_presence_preserves_planner_supplied_query_and_case_selector() {
    let state = test_state_with_enabled_skills(&["fs_basic"]);
    let mut route = base_route_result();
    route.route_reason = "capability_ref=filesystem.grep_text".into();
    route.resolved_intent = "capability_ref=filesystem.grep_text".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentPresenceCheck;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md".to_string();

    let args = assert_planner_supplied_skill_call_preserved(
        &state,
        &route,
        &LoopState::new(1),
        "check content presence",
        Some(
            "[CONTRACT_TEST_HINT]\nsemantic_kind=content_presence_check\nselector_query=release\nselector_case_insensitive=true\n[/CONTRACT_TEST_HINT]",
        ),
        Some(&route.route_reason),
        "fs_basic",
        "grep_text",
        json!({
            "action": "grep_text",
            "root": "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md",
            "query": "release",
            "case_insensitive": true
        }),
    );

    assert_eq!(
        args.get("action").and_then(Value::as_str),
        Some("grep_text")
    );
    assert_eq!(args.get("query").and_then(Value::as_str), Some("release"));
    assert_eq!(
        args.get("case_insensitive").and_then(Value::as_bool),
        Some(true)
    );
}

#[test]
fn contract_hint_preferred_doc_parse_no_longer_bypasses_agent_loop() {
    let state = test_state_with_enabled_skills(&["doc_parse"]);
    let mut route = base_route_result();
    route.route_reason = "structured_contract_hint_fast_path; contract_hint_fast_path".into();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentPresenceCheck;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md".to_string();
    let request = "[CONTRACT_TEST_HINT]\nsemantic_kind=content_presence_check\npreferred_action_ref=doc_parse.parse_doc\nselector_query=release\nselector_case_insensitive=true\n[/CONTRACT_TEST_HINT]";

    let normalized = normalize_planned_actions_with_original_and_context(
        &state,
        Some(&route),
        &LoopState::new(1),
        "check content presence using preferred parser",
        None,
        Some(request),
        None,
        Vec::new(),
    );

    assert!(normalized.is_empty());
}

#[test]
fn quoted_literal_content_presence_preserves_planner_supplied_grep_query() {
    let state = test_state_with_enabled_skills(&["fs_basic"]);
    let mut route = base_route_result();
    route.route_reason = "capability_ref=filesystem.grep_text".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::ContentPresenceCheck;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md".to_string();
    let request = "Check virtual_tools.rs for “NEEDLE_TOKEN_123”.";

    let args = assert_planner_supplied_skill_call_preserved(
        &state,
        &route,
        &LoopState::new(1),
        request,
        Some(request),
        Some(&route.route_reason),
        "fs_basic",
        "grep_text",
        json!({
            "action": "grep_text",
            "root": "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md",
            "query": "NEEDLE_TOKEN_123"
        }),
    );

    assert_eq!(
        args.get("root").and_then(Value::as_str),
        Some("scripts/nl_tests/fixtures/device_local/docs/release_checklist.md")
    );
    assert_eq!(
        args.get("query").and_then(Value::as_str),
        Some("NEEDLE_TOKEN_123")
    );
}

#[test]
fn selector_query_content_presence_preserves_planner_supplied_grep_query() {
    let state = test_state_with_enabled_skills(&["fs_basic"]);
    let mut route = base_route_result();
    route.route_reason = "capability_ref=filesystem.grep_text".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::ContentPresenceCheck;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md".to_string();
    route.route_reason =
        "capability_ref=filesystem.grep_text; selector_query=ERROR; locator_kind=path".to_string();

    let args = assert_planner_supplied_skill_call_preserved(
        &state,
        &route,
        &LoopState::new(1),
        "list matching content lines",
        Some("list matching content lines"),
        Some(&route.route_reason),
        "fs_basic",
        "grep_text",
        json!({
            "action": "grep_text",
            "root": "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md",
            "query": "ERROR"
        }),
    );

    assert_eq!(
        args.get("root").and_then(Value::as_str),
        Some("scripts/nl_tests/fixtures/device_local/docs/release_checklist.md")
    );
    assert_eq!(args.get("query").and_then(Value::as_str), Some("ERROR"));
}

#[test]
fn hidden_entries_preserves_planner_supplied_include_hidden_list_dir() {
    let state = test_state_with_enabled_skills(&["fs_basic"]);
    let mut route = base_route_result();
    route.route_reason = "capability_ref=filesystem.list_dir".into();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = OutputSemanticKind::HiddenEntriesCheck;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = ".".to_string();

    let args = assert_planner_supplied_skill_call_preserved(
        &state,
        &route,
        &LoopState::new(1),
        "check hidden entries",
        Some(
            "[CONTRACT_TEST_HINT]\nsemantic_kind=hidden_entries_check\npreferred_action_ref=fs_basic.list_dir\n[/CONTRACT_TEST_HINT]",
        ),
        Some(&route.route_reason),
        "fs_basic",
        "list_dir",
        json!({"action": "list_dir", "path": ".", "include_hidden": true}),
    );

    assert_eq!(args.get("action").and_then(Value::as_str), Some("list_dir"));
    assert_eq!(
        args.get("include_hidden").and_then(Value::as_bool),
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
    route.route_reason = "capability_ref=filesystem.list_dir".to_string();
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

    let args = assert_planner_supplied_skill_call_preserved(
        &state,
        &route,
        &LoopState::new(1),
        "inspect current workspace entries with hidden entries included",
        Some("inspect current workspace entries"),
        Some(&route.route_reason),
        "fs_basic",
        "list_dir",
        json!({
            "action": "list_dir",
            "path": root_path,
            "include_hidden": true,
            "max_entries": 3
        }),
    );

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
fn service_status_process_request_allows_planner_supplied_process_filter() {
    let state = test_state_with_enabled_skills(&["process_basic"]);
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::respond_trace();
    route.resolved_intent = "capability_ref=process.ps filter=clawd".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    let loop_state = LoopState::new(1);
    let action = AgentAction::CallSkill {
        skill: "process_basic".to_string(),
        args: json!({"action": "ps", "filter": "clawd", "limit": 200}),
    };
    let AgentAction::CallSkill { skill, args } = &action else {
        unreachable!("test action is a skill call");
    };
    assert!(
        crate::evidence_policy::capability_ref_action_policy_for_route(Some(&route), skill, args)
            .is_some_and(|policy| policy.is_allowed())
    );

    let normalized = normalize_planned_actions_with_original_and_context(
        &state,
        Some(&route),
        &loop_state,
        "check clawd process",
        Some("ordinary request text"),
        Some(&route.resolved_intent),
        None,
        vec![action],
    );

    let args = normalized
        .iter()
        .find_map(|action| {
            planned_call_is(action, "process_basic", "ps")
                .then(|| expect_planned_call(action, "process_basic", "ps"))
        })
        .expect("planner-supplied process_basic ps action should be preserved");
    assert_eq!(args.get("filter").and_then(Value::as_str), Some("clawd"));
    assert_eq!(args.get("limit").and_then(Value::as_u64), Some(200));
}

#[test]
fn service_status_process_request_without_machine_filter_leaves_empty_plan_empty() {
    let state = test_state_with_enabled_skills(&["process_basic"]);
    let mut route = base_route_result();
    route.resolved_intent = "capability_ref=process.ps".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_hint.clear();
    let loop_state = LoopState::new(1);

    let normalized = normalize_planned_actions_with_original_and_context(
        &state,
        Some(&route),
        &loop_state,
        "check clawd process",
        Some("ordinary request text"),
        Some(&route.resolved_intent),
        None,
        Vec::new(),
    );

    assert!(
        normalized.is_empty(),
        "route/user text must not synthesize an ambient process filter: {normalized:?}"
    );
}

#[test]
fn async_job_protocol_without_loop_command_does_not_parse_text_command() {
    let state = test_state_with_enabled_skills(&["process_basic", "run_cmd"]);
    let mut route = base_route_result();
    route.resolved_intent = "async_job_protocol: run `sleep 2 && echo RUSTCLAW_ASYNC_SMOKE`; adapter_result.type=pending_async_job next_step=poll_async_job".to_string();
    route.route_reason = "async_job_protocol required_job_fields=job_id|status|poll_after_seconds|expires_at|cancel_ref|message_key checkpoint_states=waiting|background".to_string();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;

    let normalized = normalize_planned_actions_with_original_and_context(
        &state,
        Some(&route),
        &LoopState::new(1),
        "start async job",
        Some("start async job"),
        Some(&route.resolved_intent),
        None,
        Vec::new(),
    );

    assert!(
        normalized.is_empty(),
        "route/user text must not synthesize async run_cmd: {normalized:?}"
    );
}

#[test]
fn async_job_protocol_injects_async_start_into_planned_run_cmd() {
    let state = test_state_with_enabled_skills(&["run_cmd"]);
    let mut route = base_route_result();
    route.resolved_intent =
        "async_job_protocol adapter_result.type=pending_async_job next_step=poll_async_job"
            .to_string();
    route.route_reason = "async_job_protocol required_job_fields=job_id|status|poll_after_seconds|expires_at|cancel_ref|message_key checkpoint_states=waiting|background".to_string();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command":"sleep 2 && echo RUSTCLAW_ASYNC_SMOKE"}),
    }];

    let normalized = normalize_planned_actions_with_original_and_context(
        &state,
        Some(&route),
        &LoopState::new(1),
        "start runtime async job",
        Some("start runtime async job"),
        Some("start runtime async job"),
        None,
        actions,
    );

    let [AgentAction::CallSkill { skill, args }] = normalized.as_slice() else {
        panic!("expected single run_cmd action, got {normalized:?}");
    };
    assert_eq!(skill, "run_cmd");
    assert_eq!(
        args.get("command").and_then(Value::as_str),
        Some("sleep 2 && echo RUSTCLAW_ASYNC_SMOKE")
    );
    assert_eq!(args.get("async_start").and_then(Value::as_bool), Some(true));
    assert_eq!(
        args.get("poll_after_seconds").and_then(Value::as_u64),
        Some(2)
    );
    assert_eq!(
        args.get("expires_in_seconds").and_then(Value::as_u64),
        Some(600)
    );
    assert_eq!(
        args.get(CLAWD_RUNTIME_ASYNC_JOB_START_ARG)
            .and_then(Value::as_str),
        Some("async_job_protocol")
    );
}

#[test]
fn contract_hint_process_basic_does_not_use_resolved_intent_as_process_filter() {
    let state = test_state_with_enabled_skills(&["process_basic"]);
    let mut route = base_route_result();
    route.route_reason = "contract_hint_fast_path".to_string();
    route.resolved_intent = "telegramd".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    route.output_contract.locator_hint.clear();
    let request = "[CONTRACT_TEST_HINT]\npreferred_action_ref=process_basic\n[/CONTRACT_TEST_HINT]";

    let normalized = normalize_planned_actions_with_original_and_context(
        &state,
        Some(&route),
        &LoopState::new(1),
        "check status",
        Some("telegramd"),
        Some(request),
        None,
        Vec::new(),
    );

    assert!(
        normalized.is_empty(),
        "contract hints and resolved_intent text must not synthesize process_basic.ps: {normalized:?}"
    );
}

#[test]
fn run_cmd_service_status_does_not_use_resolved_intent_as_process_filter() {
    let state = test_state_with_enabled_skills(&["run_cmd"]);
    let mut route = base_route_result();
    route.resolved_intent = "telegramd".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    route.output_contract.locator_hint.clear();

    let action = preferred_run_cmd_for_contract_hint(&state, &route, None)
        .expect("service status contract can use run_cmd fallback");
    let (skill, args) = planned_call(&action).expect("planned call");
    assert_eq!(skill, "run_cmd");
    let command = args
        .get("command")
        .and_then(Value::as_str)
        .expect("run command");

    assert!(command.contains("'clawd'"), "{command}");
    assert!(!command.contains("telegramd"), "{command}");
}

#[test]
fn service_status_url_request_allows_planner_supplied_http_get() {
    let state = test_state_with_enabled_skills(&["process_basic", "http_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    route.resolved_intent =
        "capability_ref=http.get url=http://127.0.0.1:8787/v1/health".to_string();
    let loop_state = LoopState::new(1);
    let action = AgentAction::CallSkill {
        skill: "http_basic".to_string(),
        args: json!({"action": "get", "url": "http://127.0.0.1:8787/v1/health"}),
    };
    let AgentAction::CallSkill { skill, args } = &action else {
        unreachable!("test action is a skill call");
    };
    assert!(
        crate::evidence_policy::capability_ref_action_policy_for_route(Some(&route), skill, args)
            .is_some_and(|policy| policy.is_allowed())
    );

    let normalized = normalize_planned_actions_with_original_and_context(
        &state,
        Some(&route),
        &loop_state,
        "observe local health URL",
        Some("health request"),
        Some(&route.resolved_intent),
        None,
        vec![action],
    );

    let args = normalized
        .iter()
        .find_map(|action| {
            planned_call_is(action, "http_basic", "get")
                .then(|| expect_planned_call(action, "http_basic", "get"))
        })
        .expect("planner-supplied http_basic get action should be preserved");
    assert_eq!(
        args.get("url").and_then(Value::as_str),
        Some("http://127.0.0.1:8787/v1/health")
    );
}

#[test]
fn service_status_url_request_without_machine_capability_leaves_empty_plan_empty() {
    let state = test_state_with_enabled_skills(&["process_basic", "http_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    route.resolved_intent = "local health request".to_string();
    let loop_state = LoopState::new(1);

    let normalized = normalize_planned_actions_with_original_and_context(
        &state,
        Some(&route),
        &loop_state,
        "observe local health URL",
        Some("check http://127.0.0.1:8787/v1/health"),
        Some(&route.resolved_intent),
        None,
        Vec::new(),
    );

    assert!(
        normalized.is_empty(),
        "user-visible URL text must not act as deterministic route authority"
    );
}

#[tokio::test]
async fn http_download_artifact_contract_reaches_planner_path() {
    let state = test_state_with_enabled_skills(&["http_basic"]);
    let task = ClaimedTask {
        task_id: "http-download-artifact-plan-round".to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: json!({ "text": "download https://example.com to document/http/download/nl-codex-parity-example.body" }).to_string(),
    };
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = OutputSemanticKind::FilesystemMutationResult;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "document/http/download/nl-codex-parity-example.body".to_string();
    route.resolved_intent = "capability_ref=http_basic.get url=https://example.com".to_string();
    let loop_state = LoopState::new(1);
    let policy = super::super::super::support::load_agent_loop_guard_policy(&state);

    let err = super::super::plan_round_actions(
        &state,
        &task,
        &route.resolved_intent,
        "download https://example.com to document/http/download/nl-codex-parity-example.body",
        &policy,
        &loop_state,
        None,
        None,
        Some(&route),
        None,
    )
    .await
    .expect_err("http download artifact should reach planner instead of pre-LLM http_basic plan");
    assert!(
        err.contains("required prompt missing"),
        "expected missing planner prompt after deterministic shortcut removal, got: {err}"
    );
    assert!(
        !err.contains("plan_deterministic_http_download_artifact"),
        "old http download deterministic fallback leaked into planner error: {err}"
    );
}

#[test]
fn service_status_task_control_marker_allows_planner_supplied_task_list() {
    let state = test_state_with_enabled_skills(&["health_check", "task_control"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    route.route_reason = "capability_ref=task_control.list".to_string();
    let loop_state = LoopState::new(1);
    let action = AgentAction::CallSkill {
        skill: "task_control".to_string(),
        args: json!({"action": "list"}),
    };
    let AgentAction::CallSkill { skill, args } = &action else {
        unreachable!("test action is a skill call");
    };
    assert!(
        crate::evidence_policy::capability_ref_action_policy_for_route(Some(&route), skill, args)
            .is_some_and(|policy| policy.is_allowed())
    );

    let normalized = normalize_planned_actions_with_original_and_context(
        &state,
        Some(&route),
        &loop_state,
        "observe current task queue",
        Some("status query"),
        Some(&route.route_reason),
        None,
        vec![action],
    );

    let args = normalized
        .iter()
        .find_map(|action| {
            planned_call_is(action, "task_control", "list")
                .then(|| expect_planned_call(action, "task_control", "list"))
        })
        .expect("planner-supplied task_control list action should be preserved");
    assert_eq!(args.as_object().map(|obj| obj.len()), Some(1));
}

#[test]
fn service_status_task_id_token_allows_planner_supplied_task_get() {
    let state = test_state_with_enabled_skills(&["health_check", "task_control"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    let task_id = "00000000-0000-4000-8000-000000000000";
    route.resolved_intent = format!("capability_ref=task_control.get task_id={task_id}");
    let loop_state = LoopState::new(1);
    let action = AgentAction::CallSkill {
        skill: "task_control".to_string(),
        args: json!({"action": "get", "task_id": task_id}),
    };
    let AgentAction::CallSkill { skill, args } = &action else {
        unreachable!("test action is a skill call");
    };
    assert!(
        crate::evidence_policy::capability_ref_action_policy_for_route(Some(&route), skill, args)
            .is_some_and(|policy| policy.is_allowed())
    );

    let normalized = normalize_planned_actions_with_original_and_context(
        &state,
        Some(&route),
        &loop_state,
        "observe task lifecycle",
        Some(&format!("query task {task_id}")),
        Some(&route.resolved_intent),
        None,
        vec![action],
    );

    let args = normalized
        .iter()
        .find_map(|action| {
            planned_call_is(action, "task_control", "get")
                .then(|| expect_planned_call(action, "task_control", "get"))
        })
        .expect("planner-supplied task_control get action should be preserved");
    assert_eq!(args.get("task_id").and_then(Value::as_str), Some(task_id));
}

#[test]
fn command_output_summary_task_id_token_allows_planner_supplied_task_get() {
    let state = test_state_with_enabled_skills(&["git_basic", "task_control"]);
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::respond_trace();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.semantic_kind = OutputSemanticKind::CommandOutputSummary;
    let task_id = "00000000-0000-4000-8000-000000000001";
    route.resolved_intent = format!("capability_ref=task_control.get task_id={task_id}");
    let loop_state = LoopState::new(1);
    let action = AgentAction::CallSkill {
        skill: "task_control".to_string(),
        args: json!({"action": "get", "task_id": task_id}),
    };
    let AgentAction::CallSkill { skill, args } = &action else {
        unreachable!("test action is a skill call");
    };
    assert!(
        crate::evidence_policy::capability_ref_action_policy_for_route(Some(&route), skill, args)
            .is_some_and(|policy| policy.is_allowed())
    );

    let normalized = normalize_planned_actions_with_original_and_context(
        &state,
        Some(&route),
        &loop_state,
        "observe task lifecycle fields",
        Some(&format!("task_id={task_id} data.lifecycle.can_poll")),
        Some(&route.resolved_intent),
        None,
        vec![action],
    );

    let args = normalized
        .iter()
        .find_map(|action| {
            planned_call_is(action, "task_control", "get")
                .then(|| expect_planned_call(action, "task_control", "get"))
        })
        .expect("planner-supplied task_control get action should be preserved");
    assert_eq!(args.get("task_id").and_then(Value::as_str), Some(task_id));
}

#[test]
fn content_presence_task_control_first_detail_allows_planner_supplied_action() {
    let state = test_state_with_enabled_skills(&["task_control"]);
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::respond_trace();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentPresenceCheck;
    route.route_reason = "capability_ref=task_control.list_with_first_detail".to_string();
    route.resolved_intent = "field_selector=lifecycle_field_presence".to_string();
    let loop_state = LoopState::new(1);
    let action = AgentAction::CallSkill {
        skill: "task_control".to_string(),
        args: json!({"action": "list_with_first_detail"}),
    };
    let AgentAction::CallSkill { skill, args } = &action else {
        unreachable!("test action is a skill call");
    };
    assert!(
        crate::evidence_policy::capability_ref_action_policy_for_route(Some(&route), skill, args)
            .is_some_and(|policy| policy.is_allowed())
    );

    let normalized = normalize_planned_actions_with_original_and_context(
        &state,
        Some(&route),
        &loop_state,
        "observe task lifecycle field presence",
        Some("task lifecycle field presence"),
        Some(&route.route_reason),
        None,
        vec![action],
    );

    let args = normalized
        .iter()
        .find_map(|action| {
            planned_call_is(action, "task_control", "list_with_first_detail")
                .then(|| expect_planned_call(action, "task_control", "list_with_first_detail"))
        })
        .expect("planner-supplied task_control list_with_first_detail action should be preserved");
    assert_eq!(args.as_object().map(|obj| obj.len()), Some(1));
}

#[test]
fn command_output_summary_does_not_shortcut_to_explicit_file_read_plan() {
    let state = test_state_with_enabled_skills(&["fs_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = OutputSemanticKind::CommandOutputSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "tmp/nl_codex_resume_smoke/note.txt".to_string();
    let loop_state = LoopState::new(1);

    let normalized = normalize_planned_actions_with_original_and_context(
        &state,
        Some(&route),
        &loop_state,
        "filesystem mutation result should not be read-only shortcut",
        Some("tmp/nl_codex_resume_smoke/note.txt"),
        None,
        None,
        Vec::new(),
    );

    assert!(
        normalized.is_empty(),
        "command-output summary routes must not synthesize fs_basic.read_text_range without planner action: {normalized:?}"
    );
}

#[test]
fn scratch_filesystem_mutation_preserves_planner_supplied_lifecycle_actions() {
    let state = test_state_with_enabled_skills(&["fs_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = OutputSemanticKind::FilesystemMutationResult;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "tmp/nl_codex_resume_smoke".to_string();
    let loop_state = LoopState::new(1);

    let actions = vec![
        AgentAction::CallSkill {
            skill: "fs_basic".to_string(),
            args: json!({"action": "make_dir", "path": "tmp/nl_codex_resume_smoke"}),
        },
        AgentAction::CallSkill {
            skill: "fs_basic".to_string(),
            args: json!({
                "action": "write_text",
                "path": "tmp/nl_codex_resume_smoke/note.txt",
                "content": "alpha\n"
            }),
        },
        AgentAction::CallSkill {
            skill: "fs_basic".to_string(),
            args: json!({
                "action": "append_text",
                "path": "tmp/nl_codex_resume_smoke/note.txt",
                "content": "beta\n"
            }),
        },
        AgentAction::CallSkill {
            skill: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": "tmp/nl_codex_resume_smoke/note.txt",
                "start_line": 1,
                "end_line": 20
            }),
        },
        AgentAction::CallSkill {
            skill: "fs_basic".to_string(),
            args: json!({
                "action": "remove_path",
                "path": "tmp/nl_codex_resume_smoke",
                "target_kind": "directory",
                "recursive": true
            }),
        },
    ];
    let normalized = normalize_planned_actions_with_original_and_context(
        &state,
        Some(&route),
        &loop_state,
        "scratch filesystem lifecycle",
        Some(
            "在 tmp/nl_codex_resume_smoke 创建目录，写 note.txt 内容 alpha，再追加 beta，读取确认两行都存在，然后删除这个临时目录；只汇总结构化结果。",
        ),
        Some(&route.output_contract.locator_hint),
        None,
        actions,
    );

    assert!(normalized.len() >= 5, "{normalized:?}");
    let make_dir = normalized
        .iter()
        .find(|action| planned_call_is(action, "fs_basic", "make_dir"))
        .expect("planner-supplied make_dir action should be preserved");
    let args = expect_planned_call(make_dir, "fs_basic", "make_dir");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some("tmp/nl_codex_resume_smoke")
    );
    let write_text = normalized
        .iter()
        .find(|action| planned_call_is(action, "fs_basic", "write_text"))
        .expect("planner-supplied write_text action should be preserved");
    let args = expect_planned_call(write_text, "fs_basic", "write_text");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some("tmp/nl_codex_resume_smoke/note.txt")
    );
    assert_eq!(args.get("content").and_then(Value::as_str), Some("alpha\n"));
    let append_text = normalized
        .iter()
        .find(|action| planned_call_is(action, "fs_basic", "append_text"))
        .expect("planner-supplied append_text action should be preserved");
    let args = expect_planned_call(append_text, "fs_basic", "append_text");
    assert_eq!(args.get("content").and_then(Value::as_str), Some("beta\n"));
    let remove_path = normalized
        .iter()
        .find(|action| planned_call_is(action, "fs_basic", "remove_path"))
        .expect("planner-supplied remove_path action should be preserved");
    let args = expect_planned_call(remove_path, "fs_basic", "remove_path");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some("tmp/nl_codex_resume_smoke")
    );
    assert_eq!(
        args.get("target_kind").and_then(Value::as_str),
        Some("directory")
    );
    assert_eq!(args.get("recursive").and_then(Value::as_bool), Some(true));
}

#[test]
fn service_status_health_check_capability_allows_planner_supplied_action() {
    let state = test_state_with_enabled_skills(&["health_check", "process_basic"]);
    let mut route = base_route_result();
    route.resolved_intent = "capability_ref=system.health_check".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    let loop_state = LoopState::new(1);

    let args = assert_planner_supplied_skill_call_preserved(
        &state,
        &route,
        &loop_state,
        "check local project service health",
        Some("health request"),
        Some(&route.resolved_intent),
        "health_check",
        "check",
        json!({"action": "check"}),
    );
    assert_eq!(args.as_object().map(|obj| obj.len()), Some(1));
}

#[test]
fn service_status_workspace_product_text_without_capability_defers_to_planner() {
    let state = test_state_with_enabled_skills(&["health_check", "process_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    let loop_state = LoopState::new(1);

    let normalized = normalize_planned_actions_with_original_and_context(
        &state,
        Some(&route),
        &loop_state,
        "check local project service health",
        Some("check RustClaw health"),
        Some(&route.resolved_intent),
        None,
        Vec::new(),
    );

    assert!(normalized.is_empty());
}

#[test]
fn service_status_health_check_recipe_uses_explicit_capability_action() {
    let state = test_state_with_enabled_skills(&["health_check", "process_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.route_reason = "capability_ref=system.health_check".to_string();
    let loop_state = LoopState::new(1);

    let args = assert_planner_supplied_skill_call_preserved(
        &state,
        &route,
        &loop_state,
        "run a structured health observation",
        Some("run a structured health observation"),
        Some(&route.route_reason),
        "health_check",
        "check",
        json!({"action": "check"}),
    );
    assert_eq!(args.as_object().map(|obj| obj.len()), Some(1));
}

#[test]
fn scalar_service_status_allows_planner_supplied_health_check() {
    let state = test_state_with_enabled_skills(&["health_check", "process_basic"]);
    let mut route = base_route_result();
    route.resolved_intent = "capability_ref=system.health_check".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    let loop_state = LoopState::new(1);

    let args = assert_planner_supplied_skill_call_preserved(
        &state,
        &route,
        &loop_state,
        "return one runtime scalar",
        Some("current runtime scalar"),
        Some(&route.resolved_intent),
        "health_check",
        "check",
        json!({"action": "check"}),
    );
    assert_eq!(args.as_object().map(|obj| obj.len()), Some(1));
}

#[test]
fn scalar_service_status_named_process_allows_planner_supplied_filter() {
    let state = test_state_with_enabled_skills(&["health_check", "process_basic"]);
    let mut route = base_route_result();
    route.resolved_intent = "capability_ref=process.ps filter=telegramd".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    let loop_state = LoopState::new(1);

    let args = assert_planner_supplied_skill_call_preserved(
        &state,
        &route,
        &loop_state,
        "check named service",
        Some("ordinary request text"),
        Some(&route.resolved_intent),
        "process_basic",
        "ps",
        json!({"action": "ps", "filter": "telegramd", "limit": 200}),
    );
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

    assert!(route_contract_defers_literal_command_to_planner(Some(
        &route
    )));
}

#[test]
fn service_status_port_request_allows_planner_supplied_port_filter() {
    let state = test_state_with_enabled_skills(&["process_basic"]);
    let mut route = base_route_result();
    route.resolved_intent = "capability_ref=process.port_list port=8787".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    let loop_state = LoopState::new(1);

    let args = assert_planner_supplied_skill_call_preserved(
        &state,
        &route,
        &loop_state,
        "check local port",
        Some("ordinary request text"),
        Some(&route.resolved_intent),
        "process_basic",
        "port_list",
        json!({"action": "port_list", "filter": "8787"}),
    );
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

    let normalized = normalize_planned_actions_with_original_and_context(
        &state,
        Some(&route),
        &loop_state,
        "observe process ranking",
        Some("看一下当前最占 CPU 的前 5 个进程，简短告诉我最值得注意的是哪个"),
        Some(&route.resolved_intent),
        None,
        Vec::new(),
    );

    assert!(
        normalized.is_empty(),
        "visible process-ranking text must not synthesize process_basic.port_list: {normalized:?}"
    );
}

#[test]
fn service_status_without_machine_target_does_not_use_system_basic_info_fallback() {
    let state = test_state_with_enabled_skills(&["system_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    let loop_state = LoopState::new(1);

    let normalized = normalize_planned_actions_with_original_and_context(
        &state,
        Some(&route),
        &loop_state,
        "observe status",
        Some("generic status"),
        Some(&route.resolved_intent),
        None,
        Vec::new(),
    );

    assert!(
        normalized.is_empty(),
        "generic service_status without machine target should leave system_basic choice to planner"
    );
}

#[test]
fn service_status_identity_field_allows_planner_supplied_runtime_status() {
    let state = test_state_with_enabled_skills(&["health_check", "system_basic"]);
    let mut route = base_route_result();
    route.resolved_intent = "capability_ref=system.runtime_status".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    let loop_state = LoopState::new(1);

    let args = assert_planner_supplied_skill_call_preserved(
        &state,
        &route,
        &loop_state,
        "observe local runtime identity",
        Some("ordinary request text"),
        Some(&route.resolved_intent),
        "system_basic",
        "runtime_status",
        json!({"action": "runtime_status"}),
    );

    assert_eq!(args.as_object().map(|obj| obj.len()), Some(1));
}

#[test]
fn service_status_generic_status_without_machine_target_defers_to_planner() {
    let state = test_state_with_enabled_skills(&["health_check", "process_basic", "system_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    let loop_state = LoopState::new(1);

    let normalized = normalize_planned_actions_with_original_and_context(
        &state,
        Some(&route),
        &loop_state,
        "observe status",
        Some("generic status"),
        Some(&route.resolved_intent),
        None,
        Vec::new(),
    );

    assert!(
        normalized.is_empty(),
        "generic service_status without machine target should leave capability choice to planner"
    );
}

#[test]
fn structured_dry_run_route_does_not_synthesize_response_without_planner_action() {
    let mut route = base_route_result();
    route.route_reason =
        "capability_ref=task_control.cancel_one dry_run=true would_mutate=false".to_string();
    route.resolved_intent = "task_control.cancel_one action=cancel_one".to_string();
    let loop_state = LoopState::new(1);

    let normalized = normalize_planned_actions_with_original_and_context(
        &test_state_with_enabled_skills(&["task_control"]),
        Some(&route),
        &loop_state,
        "dry-run task cancel",
        Some("dry-run task cancel"),
        Some(&route.route_reason),
        None,
        Vec::new(),
    );

    assert!(
        normalized.is_empty(),
        "dry-run route machine tokens must not synthesize a response without planner action: {normalized:?}"
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
        crate::AskMode::act_with_chat_finalizer(),
        true,
        OutputResponseShape::Strict,
    );
    route.route_reason = "capability_ref=archive.pack".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::None;
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
fn date_run_cmd_rewrites_to_system_basic_current_time() {
    let state = test_state_with_enabled_skills(&["system_basic", "run_cmd"]);
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": "date '+%Y-%m-%d %H:%M:%S %Z'"}),
    }];

    let rewritten = rewrite_readonly_runtime_status_run_cmd_to_system_basic(&state, None, actions);
    let args = expect_planned_call(&rewritten[0], "system_basic", "runtime_status");

    assert_eq!(
        args.get("kind").and_then(Value::as_str),
        Some("current_time")
    );
}

#[test]
fn runtime_status_run_cmd_rewrite_preserves_literal_command_flag() {
    let state = test_state_with_enabled_skills(&["system_basic", "run_cmd"]);
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({
            "command": "pwd",
            CLAWD_LITERAL_COMMAND_ARG: true,
        }),
    }];

    let rewritten = rewrite_readonly_runtime_status_run_cmd_to_system_basic(&state, None, actions);

    match &rewritten[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "run_cmd");
            assert_eq!(args.get("command").and_then(Value::as_str), Some("pwd"));
            assert_eq!(args.get(CLAWD_LITERAL_COMMAND_ARG), Some(&json!(true)));
        }
        other => panic!("expected literal run_cmd action, got {other:?}"),
    }
}

#[test]
fn runtime_status_run_cmd_rewrite_rejects_shell_control_commands() {
    let state = test_state_with_enabled_skills(&["system_basic", "run_cmd"]);
    let command = "date; rm -rf /tmp/rustclaw-noop";
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": command}),
    }];

    let rewritten = rewrite_readonly_runtime_status_run_cmd_to_system_basic(&state, None, actions);

    match &rewritten[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "run_cmd");
            assert_eq!(args.get("command").and_then(Value::as_str), Some(command));
        }
        other => panic!("expected unchanged run_cmd action, got {other:?}"),
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
    route.ask_mode = crate::AskMode::act_with_chat_finalizer();
    route.resolved_intent = "Inspect the archive contents without unpacking it.".to_string();
    route.route_reason = "capability_ref=archive.list".to_string();
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

    let args = assert_planner_supplied_skill_call_preserved(
        &state,
        &route,
        &loop_state,
        "Inspect the archive",
        Some("Inspect the archive contents without unpacking it."),
        Some(&route.route_reason),
        "archive_basic",
        "list",
        json!({
            "action": "list",
            "archive": "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip"
        }),
    );

    assert_eq!(args.get("action").and_then(Value::as_str), Some("list"));
    assert_eq!(
        args.get("archive").and_then(Value::as_str),
        Some("scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip")
    );
}

#[test]
fn archive_read_contract_plans_direct_member_read() {
    let state = test_state_with_enabled_skills(&["archive_basic"]);
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::act_with_chat_finalizer();
    route.resolved_intent =
        "Read member notes.txt from scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip"
            .to_string();
    route.route_reason = "capability_ref=archive.read".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::ArchiveRead;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip | notes.txt".to_string();
    let loop_state = LoopState::new(1);

    let args = assert_planner_supplied_skill_call_preserved(
        &state,
        &route,
        &loop_state,
        "read archive member",
        Some(
            "Read member notes.txt from scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip",
        ),
        Some(&route.route_reason),
        "archive_basic",
        "read",
        json!({
            "action": "read",
            "archive": "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip",
            "member": "notes.txt"
        }),
    );

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
    route.ask_mode = crate::AskMode::act_with_chat_finalizer();
    route.resolved_intent = format!("Read notes.txt from {archive}");
    route.route_reason = "capability_ref=archive.read".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::ArchiveRead;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.locator_hint = format!("{archive} | notes.txt");
    let loop_state = LoopState::new(1);

    let args = assert_planner_supplied_skill_call_preserved(
        &state,
        &route,
        &loop_state,
        "read archive member",
        Some(&format!("Read member notes.txt from {archive}")),
        Some(&route.route_reason),
        "archive_basic",
        "read",
        json!({
            "action": "read",
            "archive": archive,
            "member": "notes.txt"
        }),
    );

    assert_eq!(args.get("archive").and_then(Value::as_str), Some(archive));
    assert_eq!(
        args.get("member").and_then(Value::as_str),
        Some("notes.txt")
    );
}

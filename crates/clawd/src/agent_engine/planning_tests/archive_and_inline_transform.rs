use super::*;

#[test]
fn archive_read_capability_ref_allows_planner_supplied_member_args() {
    let archive = "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip";
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::act_with_chat_finalizer();
    route.resolved_intent = "capability_ref=archive.read".to_string();
    route.route_reason = "capability_ref=archive.read".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.locator_hint = archive.to_string();

    let policy = crate::evidence_policy::capability_ref_action_policy_for_route(
        Some(&route),
        "archive_basic",
        &json!({
            "action": "read",
            "archive": archive,
            "member": "notes.txt",
        }),
    )
    .expect("archive.read capability ref should expose archive_basic.read");
    assert!(policy.is_allowed(), "{policy:?}");
    assert!(policy.action_matches_preferred(), "{policy:?}");
}

#[test]
fn archive_read_capability_ref_uses_policy_not_archive_read_semantic_kind() {
    let archive = "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip";
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::act_with_chat_finalizer();
    route.resolved_intent = "capability_ref=archive.read".to_string();
    route.route_reason = "capability_ref=archive.read".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.locator_hint = "test_bundle.zip | notes.txt".to_string();

    let policy = crate::evidence_policy::capability_ref_action_policy_for_route(
        Some(&route),
        "archive_basic",
        &json!({
            "action": "read",
            "archive": archive,
            "member": "notes.txt",
        }),
    )
    .expect("archive.read capability ref should work without ArchiveRead semantic kind");
    assert!(policy.is_allowed(), "{policy:?}");
    assert!(policy.action_matches_preferred(), "{policy:?}");
}

#[test]
fn archive_read_semantic_kind_without_capability_ref_does_not_expose_action_refs() {
    let archive = "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip";
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::act_with_chat_finalizer();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::ArchiveRead;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.locator_hint = format!("{archive} | notes.txt");

    assert_eq!(
        crate::evidence_policy::capability_ref_action_refs_for_route(&route, false).len(),
        0,
        "ArchiveRead output marker alone must not choose archive.read before the planner"
    );
}

#[test]
fn archive_read_structural_member_target_waits_for_planner_capability_ref() {
    let archive = "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip";
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::act_with_chat_finalizer();
    route.resolved_intent =
        format!("Read the notes.txt content from archive {archive} and output only it");
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.locator_hint = archive.to_string();

    assert!(
        crate::evidence_policy::capability_ref_action_refs_for_route(&route, false).is_empty(),
        "structural archive/member text without a machine capability_ref must be left to the planner"
    );
}

#[test]
fn archive_read_contract_rejects_unsafe_member_locator() {
    assert!(!super::super::directory_unique_entry::archive_member_path_is_safe("../secret.txt"));
    assert!(!super::super::directory_unique_entry::archive_member_path_is_safe("/tmp/secret.txt"));
    assert!(super::super::directory_unique_entry::archive_member_path_is_safe("notes.txt"));
}

#[test]
fn archive_database_aggregate_capability_refs_allow_structured_observation_actions() {
    let archive = "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip";
    let db_path = "scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite";
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::act_with_chat_finalizer();
    route.resolved_intent =
        "capability_ref=archive.list capability_ref=archive.read capability_ref=database.list_tables"
            .to_string();
    route.route_reason =
        "capability_ref=archive.list capability_ref=archive.read capability_ref=database.list_tables"
            .to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.locator_hint = format!("{archive} | {db_path}");

    let list_policy = crate::evidence_policy::capability_ref_action_policy_for_route(
        Some(&route),
        "archive_basic",
        &json!({"action": "list", "archive": archive}),
    )
    .expect("archive.list capability ref should expose archive_basic.list");
    assert!(list_policy.is_allowed(), "{list_policy:?}");
    assert!(list_policy.action_matches_preferred(), "{list_policy:?}");

    let read_policy = crate::evidence_policy::capability_ref_action_policy_for_route(
        Some(&route),
        "archive_basic",
        &json!({"action": "read", "archive": archive, "member": "notes.txt"}),
    )
    .expect("archive.read capability ref should expose archive_basic.read");
    assert!(read_policy.is_allowed(), "{read_policy:?}");
    assert!(read_policy.action_matches_preferred(), "{read_policy:?}");

    let db_policy = crate::evidence_policy::capability_ref_action_policy_for_route(
        Some(&route),
        "db_basic",
        &json!({"action": "list_tables", "db_path": db_path}),
    )
    .expect("database.list_tables capability ref should expose db_basic.list_tables");
    assert!(db_policy.is_allowed(), "{db_policy:?}");
    assert!(db_policy.action_matches_preferred(), "{db_policy:?}");
}

#[test]
fn archive_database_aggregate_without_capability_refs_does_not_expose_action_refs() {
    let archive = "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip";
    let db_path = "scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite";
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::act_with_chat_finalizer();
    route.resolved_intent =
        "llm_failed_existing_path_observation_fallback; explicit_existing_path_observation"
            .to_string();
    route.route_reason = "auto_locator_suppressed_multiple_explicit_paths".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.locator_hint = format!("{archive} | {db_path}");

    assert!(
        crate::evidence_policy::capability_ref_action_refs_for_route(&route, false).is_empty(),
        "content-excerpt fallback route must not choose compound archive/database skills without capability refs"
    );
}

#[test]
fn transform_action_alias_and_sort_args_normalize_to_transform_data_ops() {
    let actions = vec![AgentAction::CallTool {
        tool: "transform".to_string(),
        args: json!({
            "action": "transform",
            "data": [
                {"name": "alpha", "score": 7},
                {"name": "beta", "score": 12}
            ],
            "sort_by": "score",
            "order": "desc",
            "output_format": "md_table"
        }),
    }];

    let normalized = normalize_transform_schema_aliases(actions);

    let args = expect_planned_call(&normalized[0], "transform", "transform_data");
    assert_eq!(
        args.get("output_format").and_then(Value::as_str),
        Some("md_table")
    );
    let ops = args
        .get("ops")
        .and_then(Value::as_array)
        .expect("ops array");
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].get("op").and_then(Value::as_str), Some("sort"));
    assert_eq!(ops[0].get("by").and_then(Value::as_str), Some("score"));
    assert_eq!(ops[0].get("order").and_then(Value::as_str), Some("desc"));
    assert!(args.get("sort_by").is_none());
}

#[test]
fn transform_markdown_output_format_alias_normalizes_to_md_table() {
    let actions = vec![AgentAction::CallSkill {
        skill: "transform".to_string(),
        args: json!({
            "action": "transform_data",
            "records": [
                {"name": "alpha", "score": 7},
                {"name": "beta", "score": 12}
            ],
            "ops": [{"op": "sort", "by": "score", "order": "desc"}],
            "output_format": "markdown"
        }),
    }];

    let normalized = normalize_transform_schema_aliases(actions);

    let args = expect_planned_call(&normalized[0], "transform", "transform_data");
    assert_eq!(
        args.get("output_format").and_then(Value::as_str),
        Some("md_table")
    );
}

#[tokio::test]
async fn inline_json_transform_reaches_planner_path() {
    let state = test_state_with_enabled_skills(&["transform"]);
    let request = r#"{"action":"transform_data","data":[{"name":"alpha","score":7},{"name":"beta","score":12}],"ops":[{"op":"filter","where":{"field":"score","gte":7}}]}"#;
    let task = ClaimedTask {
        task_id: "inline-transform-plan-round".to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: json!({ "text": request }).to_string(),
    };
    let mut route = base_route_result();
    route.resolved_intent = request.to_string();
    route.route_reason = "capability_ref=transform.transform_data".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    let loop_state = LoopState::new(1);
    let policy = super::super::super::support::load_agent_loop_guard_policy(&state);

    let err = super::super::plan_round_actions(
        &state,
        &task,
        request,
        request,
        &policy,
        &loop_state,
        None,
        None,
        Some(&route),
        None,
    )
    .await
    .expect_err("inline transform should reach planner instead of pre-LLM transform plan");
    assert!(
        err.contains("required prompt missing"),
        "expected missing planner prompt after deterministic shortcut removal, got: {err}"
    );
    assert!(
        !err.contains("plan_deterministic_inline_json_transform"),
        "old inline transform deterministic fallback leaked into planner error: {err}"
    );
}

#[test]
fn planner_prompt_contract_guard_allows_present_compact_contract_block() {
    let mut route = base_route_result();
    route.output_contract.semantic_kind = OutputSemanticKind::FileNames;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    let contract_line =
        crate::evidence_policy::compact_prompt_line_for_route(&route).expect("contract line");
    let prompt = format!("System\n{contract_line}\nUser");

    ensure_required_contract_block_present(Some(&route), &prompt).expect("contract present");
}

#[test]
fn planner_prompt_contract_guard_fails_closed_when_compact_contract_block_missing() {
    let mut route = base_route_result();
    route.output_contract.semantic_kind = OutputSemanticKind::FileNames;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;

    let err = ensure_required_contract_block_present(Some(&route), "System\nUser")
        .expect_err("missing contract block should fail closed");

    assert!(err.contains("prompt_budget_error"));
    assert!(err.contains("contract_line_hash="));
}

#[test]
fn rewrite_extract_field_field_alias_to_field_path() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "extract_field",
            "path": "/tmp/config.toml",
            "field": "tools.allow_sudo"
        }),
    }];
    let out = rewrite_extract_field_alias_args(actions);
    match &out[0] {
        AgentAction::CallSkill { args, .. } => {
            assert_eq!(
                args.get("field_path").and_then(|value| value.as_str()),
                Some("tools.allow_sudo")
            );
            assert!(args.get("field").is_none());
        }
        other => panic!("expected call_skill, got {other:?}"),
    }
}

#[test]
fn rewrite_extract_field_keeps_existing_field_path() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "extract_field",
            "path": "/tmp/config.toml",
            "field": "tools.allow_sudo",
            "field_path": "tools.allow_path_outside_workspace"
        }),
    }];
    let out = rewrite_extract_field_alias_args(actions);
    match &out[0] {
        AgentAction::CallSkill { args, .. } => {
            assert_eq!(
                args.get("field_path").and_then(|value| value.as_str()),
                Some("tools.allow_path_outside_workspace")
            );
            assert_eq!(
                args.get("field").and_then(|value| value.as_str()),
                Some("tools.allow_sudo")
            );
        }
        other => panic!("expected call_skill, got {other:?}"),
    }
}

#[test]
fn rewrite_extract_field_file_path_alias_to_path() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "extract_field",
            "file_path": "/tmp/config.toml",
            "field_path": "tools.allow_sudo"
        }),
    }];
    let out = rewrite_extract_field_alias_args(actions);
    match &out[0] {
        AgentAction::CallSkill { args, .. } => {
            assert_eq!(
                args.get("path").and_then(|value| value.as_str()),
                Some("/tmp/config.toml")
            );
            assert!(args.get("file_path").is_none());
        }
        other => panic!("expected call_skill, got {other:?}"),
    }
}

#[test]
fn rewrite_extract_field_target_alias_to_path() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "extract_field",
            "target": "/tmp/config.toml",
            "field_path": "tools.allow_sudo"
        }),
    }];
    let out = rewrite_extract_field_alias_args(actions);
    match &out[0] {
        AgentAction::CallSkill { args, .. } => {
            assert_eq!(
                args.get("path").and_then(|value| value.as_str()),
                Some("/tmp/config.toml")
            );
            assert!(args.get("target").is_none());
        }
        other => panic!("expected call_skill, got {other:?}"),
    }
}

#[test]
fn extract_field_rewrites_bare_manifest_to_shallow_candidate_with_field() {
    let root = TempDirGuard::new("structured_manifest_candidate");
    fs::write(
        root.path.join("package.json"),
        r#"{"dependencies":{"left-pad":"1.0.0"}}"#,
    )
    .expect("write root package");
    fs::create_dir_all(root.path.join("UI")).expect("create ui");
    fs::write(
        root.path.join("UI/package.json"),
        r#"{"name":"react-example"}"#,
    )
    .expect("write ui package");
    fs::create_dir_all(root.path.join("services/wa-web-bridge")).expect("create service");
    fs::write(
        root.path.join("services/wa-web-bridge/package.json"),
        r#"{"name":"wa-web-bridge"}"#,
    )
    .expect("write service package");

    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    let root_package = root.path.join("package.json");
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "extract_field",
            "path": root_package.display().to_string(),
            "field_path": "name"
        }),
    }];

    let normalized = super::super::normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "读取 package.json 里的 name 字段",
        None,
        actions,
    );
    let args = expect_planned_call(&normalized[0], "config_basic", "read_field");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(root.path.join("UI/package.json").to_string_lossy().as_ref())
    );
}

#[test]
fn extract_field_rewrites_workspace_cargo_package_field_to_current_package_manifest() {
    let root = TempDirGuard::new("workspace_cargo_candidate");
    fs::write(
        root.path.join("Cargo.toml"),
        r#"[workspace]
members = ["crates/other", "crates/clawd"]
"#,
    )
    .expect("write workspace cargo");
    fs::create_dir_all(root.path.join("crates/other")).expect("create other");
    fs::write(
        root.path.join("crates/other/Cargo.toml"),
        r#"[package]
name = "other"
"#,
    )
    .expect("write other cargo");
    fs::create_dir_all(root.path.join("crates/clawd")).expect("create clawd");
    fs::write(
        root.path.join("crates/clawd/Cargo.toml"),
        r#"[package]
name = "clawd"
"#,
    )
    .expect("write clawd cargo");

    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    let root_cargo = root.path.join("Cargo.toml");
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "extract_field",
            "path": root_cargo.display().to_string(),
            "field_path": "package.name"
        }),
    }];

    let normalized = super::super::normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "读取 Cargo.toml 的 package.name",
        None,
        actions,
    );
    let args = expect_planned_call(&normalized[0], "config_basic", "read_field");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(
            root.path
                .join("crates/clawd/Cargo.toml")
                .to_string_lossy()
                .as_ref()
        )
    );
}

#[test]
fn extract_field_keeps_root_manifest_when_auto_locator_is_workspace_root_scope() {
    let root = TempDirGuard::new("root_scope_manifest_binding");
    let root_package = root.path.join("package.json");
    fs::write(
        &root_package,
        r#"{"dependencies":{"@xdevplatform/xurl":"^1.0.3"}}"#,
    )
    .expect("write root package");
    fs::create_dir_all(root.path.join("UI")).expect("create ui");
    fs::write(
        root.path.join("UI/package.json"),
        r#"{"name":"react-example"}"#,
    )
    .expect("write ui package");
    let root_cargo = root.path.join("Cargo.toml");
    fs::write(
        &root_cargo,
        r#"[workspace]
members = ["crates/clawd"]

[workspace.package]
version = "0.1.7"

[workspace.dependencies]
toml = "0.8"
reqwest = { version = "0.12" }
"#,
    )
    .expect("write workspace cargo");
    fs::create_dir_all(root.path.join("crates/clawd")).expect("create clawd");
    fs::write(
        root.path.join("crates/clawd/Cargo.toml"),
        r#"[package]
name = "clawd"
"#,
    )
    .expect("write member cargo");

    let mut state = test_state_with_enabled_skills(&["system_basic", "config_basic"]);
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();
    route.output_contract.delivery_required = false;
    route.resolved_intent =
        "Read root package.json name and root Cargo.toml package.name".to_string();
    let root_scope = root.path.display().to_string();
    let actions = vec![
        AgentAction::CallTool {
            tool: "system_basic".to_string(),
            args: json!({
                "action": "extract_field",
                "path": "package.json",
                "field_path": "name",
                "format": "json",
            }),
        },
        AgentAction::CallTool {
            tool: "system_basic".to_string(),
            args: json!({
                "action": "extract_field",
                "path": "Cargo.toml",
                "field_path": "package.name",
                "format": "toml",
            }),
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
        &LoopState::new(1),
        "Read root package.json name and root Cargo.toml package.name",
        Some(&root_scope),
        actions,
    );
    let read_paths = normalized
        .iter()
        .filter_map(|action| {
            let args = match action {
                AgentAction::CallSkill { skill, args }
                | AgentAction::CallTool { tool: skill, args }
                    if skill == "config_basic"
                        && args.get("action").and_then(Value::as_str) == Some("read_field") =>
                {
                    args
                }
                _ => return None,
            };
            args.get("path").and_then(Value::as_str).map(|raw| {
                let path = Path::new(raw);
                if path.is_absolute() {
                    path.to_path_buf()
                } else {
                    root.path.join(path)
                }
            })
        })
        .collect::<Vec<_>>();

    assert_eq!(read_paths.len(), 2, "normalized actions: {normalized:?}");
    assert_eq!(read_paths[0], root_package);
    assert_eq!(read_paths[1], root_cargo);
}

#[test]
fn extract_field_rewrites_workspace_cargo_package_version_to_workspace_package_version() {
    let root = TempDirGuard::new("workspace_cargo_version");
    fs::write(
        root.path.join("Cargo.toml"),
        r#"[workspace]
members = ["crates/clawd"]

[workspace.package]
version = "0.1.7"

[workspace.dependencies]
toml = "0.8"
reqwest = { version = "0.12" }
"#,
    )
    .expect("write workspace cargo");

    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RecentScalarEqualityCheck;
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    let root_cargo = root.path.join("Cargo.toml");
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "extract_field",
            "path": root_cargo.display().to_string(),
            "field_path": "package.version"
        }),
    }];

    let normalized = super::super::normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "Read workspace package version from Cargo.toml",
        Some(root_cargo.to_string_lossy().as_ref()),
        actions,
    );
    let args = expect_planned_call(&normalized[0], "config_basic", "read_field");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(root_cargo.to_string_lossy().as_ref())
    );
    assert_eq!(
        args.get("field_path").and_then(Value::as_str),
        Some("workspace.package.version")
    );
}

#[test]
fn config_basic_read_field_rewrites_workspace_cargo_package_version_to_workspace_package_version() {
    let root = TempDirGuard::new("config_workspace_cargo_version");
    fs::write(
        root.path.join("Cargo.toml"),
        r#"[workspace]
members = ["crates/clawd"]

[workspace.package]
version = "0.1.7"
"#,
    )
    .expect("write workspace cargo");

    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RecentScalarEqualityCheck;
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    let root_cargo = root.path.join("Cargo.toml");
    let actions = vec![AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args: json!({
            "action": "read_field",
            "path": root_cargo.display().to_string(),
            "field_path": "package.version"
        }),
    }];

    let normalized = super::super::normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "Read Cargo.toml version and answer as `version=<value>` only.",
        Some(root_cargo.to_string_lossy().as_ref()),
        actions,
    );
    let args = expect_planned_call(&normalized[0], "config_basic", "read_field");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(root_cargo.to_string_lossy().as_ref())
    );
    assert_eq!(
        args.get("field_path").and_then(Value::as_str),
        Some("workspace.package.version")
    );
}

#[test]
fn active_clarify_scalar_field_followup_rewrites_text_read_to_read_field() {
    let root = TempDirGuard::new("active_clarify_scalar_field_followup");
    let package = root.path.join("package.json");
    fs::write(&package, r#"{"name":"rustclaw","version":"0.1.7"}"#).expect("write package");

    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.resolved_intent =
        "Continue the previous request that was waiting for clarification: 读一下那个文件里的名字字段，只输出值\n[RESOLVED_INTENT]\n读取指定文件中的名字字段（name），仅输出该字段的值\nUser now provides the missing target or content: package.json"
            .to_string();
    route.route_reason =
        "active_clarify_locator_reply_fast_path; preserve_active_clarify_output_contract"
            .to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "package.json".to_string();
    route.output_contract.requires_content_evidence = true;
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "read_text_range",
            "path": package.display().to_string(),
            "mode": "head",
            "n": 120
        }),
    }];

    let normalized = normalize_planned_actions_with_original_and_context(
        &state,
        Some(&route),
        &LoopState::new(1),
        "package.json",
        Some("package.json"),
        None,
        Some(package.to_string_lossy().as_ref()),
        actions,
    );
    let args = expect_planned_call(&normalized[0], "config_basic", "read_field");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(package.to_string_lossy().as_ref())
    );
    assert_eq!(args.get("field_path").and_then(Value::as_str), Some("name"));
}

#[test]
fn active_clarify_scalar_candidate_respond_rewrites_to_read_field_evidence() {
    let root = TempDirGuard::new("active_clarify_scalar_candidate_respond");
    let package = root.path.join("package.json");
    fs::write(&package, r#"{"name":"rustclaw","private":true}"#).expect("write package");

    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.route_reason =
        "active_clarify_locator_reply_fast_path; active_clarify_fast_path_scalar_field_value_contract_repair"
            .to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "package.json".to_string();
    route.output_contract.requires_content_evidence = true;
    let mut loop_state = LoopState::new(3);
    loop_state.has_tool_or_skill_output = true;
    let actions = vec![AgentAction::Respond {
        content: "rustclaw".to_string(),
    }];

    let normalized = normalize_planned_actions_with_original_and_context(
        &state,
        Some(&route),
        &loop_state,
        "package.json",
        Some("package.json"),
        None,
        Some(package.to_string_lossy().as_ref()),
        actions,
    );

    let args = expect_planned_call(&normalized[0], "config_basic", "read_field");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(package.to_string_lossy().as_ref())
    );
    assert_eq!(args.get("field_path").and_then(Value::as_str), Some("name"));
}

#[test]
fn active_clarify_scalar_candidate_respond_keeps_ambiguous_value() {
    let root = TempDirGuard::new("active_clarify_scalar_candidate_ambiguous");
    let package = root.path.join("package.json");
    fs::write(&package, r#"{"name":"rustclaw","alias":"rustclaw"}"#).expect("write package");

    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.route_reason =
        "active_clarify_locator_reply_fast_path; active_clarify_fast_path_scalar_field_value_contract_repair"
            .to_string();
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "package.json".to_string();
    route.output_contract.requires_content_evidence = true;
    let mut loop_state = LoopState::new(3);
    loop_state.has_tool_or_skill_output = true;
    let actions = vec![AgentAction::Respond {
        content: "rustclaw".to_string(),
    }];

    let normalized = normalize_planned_actions_with_original_and_context(
        &state,
        Some(&route),
        &loop_state,
        "package.json",
        Some("package.json"),
        None,
        Some(package.to_string_lossy().as_ref()),
        actions,
    );

    assert!(matches!(
        normalized.as_slice(),
        [AgentAction::Respond { content }] if content == "rustclaw"
    ));
}

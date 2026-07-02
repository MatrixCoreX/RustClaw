use super::*;
use crate::agent_engine::planning::preferred_structured_action_for_contract_hint;

#[test]
fn rustclaw_main_config_content_excerpt_direct_guard_prefers_config_basic_guard() {
    let state = test_state();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();
    let actions = vec![AgentAction::CallTool {
        tool: "config_edit".to_string(),
        args: json!({
            "action": "guard_config",
            "path": "/home/guagua/rustclaw/configs/config.toml",
            "format": "toml",
        }),
    }];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "Assess the main config with structured current-task evidence.",
        Some("/home/guagua/rustclaw/configs/config.toml"),
        actions,
    );

    let args = expect_planned_call(&normalized[0], "config_basic", "guard_rustclaw_config");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some("configs/config.toml")
    );
    assert_eq!(args.get("format").and_then(Value::as_str), Some("toml"));
}

#[test]
fn rustclaw_main_config_content_excerpt_tail_read_stays_bounded_read() {
    let state = test_state();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "read_text_range",
            "path": "/home/guagua/rustclaw/configs/config.toml",
            "mode": "tail",
            "n": 5,
        }),
    }];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "Summarize the bounded tail excerpt.",
        Some("/home/guagua/rustclaw/configs/config.toml"),
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "read_text_range");
    assert_eq!(args.get("mode").and_then(Value::as_str), Some("tail"));
    assert_eq!(args.get("n").and_then(Value::as_u64), Some(5));
}

#[test]
fn schema_alias_normalization_uses_contract_field_selector_not_resolved_intent() {
    let state = test_state();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "package.json".to_string();
    route.resolved_intent = "read package.name from package.json".to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "read_range",
            "path": "package.json",
            "mode": "head",
            "n": 120,
        }),
    }];

    let normalized =
        normalize_action_schema_aliases(&state, Some(&route), "", None, actions.clone());
    let args = expect_planned_call(&normalized[0], "system_basic", "read_range");
    assert!(args.get("field_path").is_none());

    route
        .output_contract
        .self_extension
        .structured_field_selector = Some("package.name".to_string());
    let normalized = normalize_action_schema_aliases(&state, Some(&route), "", None, actions);
    let args = expect_planned_call(&normalized[0], "config_basic", "read_field");
    assert_eq!(
        args.get("field_path").and_then(Value::as_str),
        Some("package.name")
    );
}

#[test]
fn config_risk_assessment_rewrites_registry_head_read_to_guard_config() {
    let state = test_state();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::ConfigRiskAssessment;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/skills_registry.toml".to_string();
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "read_text_range",
            "path": "/home/guagua/rustclaw/configs/skills_registry.toml",
            "mode": "head",
            "n": 120,
        }),
    }];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "Structured RustClaw registry risk assessment.",
        Some("/home/guagua/rustclaw/configs/skills_registry.toml"),
        actions,
    );

    let args = expect_planned_call(&normalized[0], "config_basic", "guard_rustclaw_config");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some("configs/skills_registry.toml")
    );
    assert_eq!(args.get("format").and_then(Value::as_str), Some("toml"));
    assert!(
        normalized
            .iter()
            .all(|action| !planned_call_is(action, "fs_basic", "read_text_range")),
        "normalized actions: {normalized:?}"
    );
}

#[test]
fn scalar_structured_field_contract_rewrites_broad_read_to_read_field() {
    let root = TempDirGuard::new("structured_scalar_workspace_deps");
    let root_cargo = root.path.join("Cargo.toml");
    fs::write(
        &root_cargo,
        "[workspace]\n[workspace.dependencies]\ntoml = \"0.8\"\n",
    )
    .expect("write workspace Cargo.toml");
    let state = test_state();
    let root_cargo = root_cargo.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_cargo.clone();
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": root_cargo,
                "mode": "head",
                "n": 500,
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["step_1".to_string()],
        },
    ];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "Read workspace.dependencies.toml from Cargo.toml and output only the value.",
        None,
        actions,
    );

    let args = expect_planned_call(&normalized[0], "config_basic", "read_field");
    assert_eq!(
        args.get("field_path").and_then(Value::as_str),
        Some("workspace.dependencies.toml")
    );
}

#[test]
fn unresolved_locator_marker_preserves_terminal_respond_plan() {
    let root = TempDirGuard::new("unresolved_locator_terminal_respond");
    let package = root.path.join("package.json");
    fs::write(&package, r#"{"name":"fixture","version":"0.1.0"}"#).expect("write package");
    let docs = root.path.join("docs");
    fs::create_dir_all(&docs).expect("create docs");
    let service_notes = docs.join("service_notes.md");
    let release_checklist = docs.join("release_checklist.md");
    fs::write(&service_notes, "Service Notes\n").expect("write service notes");
    fs::write(&release_checklist, "Release Checklist\n").expect("write release checklist");
    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::FileBasename;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.route_reason =
        "state_patch.deictic_reference=missing_locator; clarify_reason_code:missing_read_target"
            .to_string();
    route.resolved_intent =
        "Return only the unresolved file target after confirmation.".to_string();
    let answer = "confirm the target scope".to_string();
    let actions = vec![AgentAction::Respond {
        content: answer.clone(),
    }];
    let plan_context = format!(
        "{}\n{}\n{}",
        package.display(),
        service_notes.display(),
        release_checklist.display()
    );

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "current request requires unresolved target confirmation",
        Some(&plan_context),
        actions,
    );

    assert_eq!(normalized.len(), 1, "normalized actions: {normalized:?}");
    match &normalized[0] {
        AgentAction::Respond { content } => assert_eq!(content, &answer),
        other => panic!("expected terminal respond to be preserved, got {other:?}"),
    }
}

#[test]
fn scalar_structured_field_contract_infers_single_field_from_structural_candidate() {
    let root = TempDirGuard::new("structured_scalar_field_candidate_plan");
    let root_package = root.path.join("package.json");
    fs::write(&root_package, r#"{"dependencies":{"vite":"latest"}}"#).expect("write root");
    let fixture_dir = root.path.join("fixtures");
    fs::create_dir_all(&fixture_dir).expect("create fixture dir");
    let fixture_package = fixture_dir.join("package.json");
    fs::write(
        &fixture_package,
        r#"{"name":"rustclaw-nl-fixture","dependencies":{}}"#,
    )
    .expect("write fixture");
    let root_package_path = root_package.display().to_string();
    let fixture_package_path = fixture_package.display().to_string();
    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "package.json".to_string();
    route.route_reason =
        "llm_semantic_contract_repair:single_path_field_extraction_semantic_kind_none_is_valid"
            .to_string();
    route.resolved_intent =
        "Extract and output only the value of the name field from package.json".to_string();
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": root_package_path,
                "mode": "head",
                "n": 500,
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["step_1".to_string()],
        },
    ];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "package.json",
        Some(&root_package.display().to_string()),
        actions,
    );

    assert_eq!(normalized.len(), 1, "normalized actions: {normalized:?}");
    let actual = &normalized[0];
    let args = expect_planned_call(actual, "config_basic", "read_field");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(fixture_package_path.as_str()),
        "unexpected normalized action: {actual:?}"
    );
    assert_eq!(args.get("field_path").and_then(Value::as_str), Some("name"));
}

#[test]
fn scalar_structured_field_contract_rewrites_key_listing_to_read_field() {
    let root = TempDirGuard::new("structured_scalar_field_list_keys_plan");
    let config_dir = root.path.join("configs");
    fs::create_dir_all(&config_dir).expect("create config dir");
    let config = config_dir.join("app_config.toml");
    fs::write(&config, "[app]\nport = 8787\n").expect("write config");
    let config_path = config.display().to_string();
    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = config_path.clone();
    route.resolved_intent = format!("Read app.port from {config_path} and output only the value.");
    let actions = vec![
        AgentAction::CallTool {
            tool: "config_basic".to_string(),
            args: json!({
                "action": "list_keys",
                "path": config_path,
                "max_keys": 1000,
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
    ];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "Read app.port from configs/app_config.toml and output only the value.",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 1, "normalized actions: {normalized:?}");
    let args = expect_planned_call(&normalized[0], "config_basic", "read_field");
    assert_eq!(
        args.get("field_path").and_then(Value::as_str),
        Some("app.port")
    );
}

#[test]
fn scalar_structured_keys_repair_marker_rewrites_key_listing_to_read_field() {
    let root = TempDirGuard::new("structured_keys_scalar_marker_plan");
    let package = root.path.join("package.json");
    fs::write(&package, r#"{"name":"fixture","dependencies":{}}"#).expect("write package");
    let package_path = package.display().to_string();
    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "package.json".to_string();
    route.route_reason =
        "llm_semantic_contract_repair:structured_keys_scalar_response_requires_field_value"
            .to_string();
    route.resolved_intent = "Extract name field value from package.json".to_string();
    let actions = vec![AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args: json!({
            "action": "list_keys",
            "path": package_path,
            "max_keys": 1000,
        }),
    }];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "去 package.json 里把 name 的值回给我",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 1, "normalized actions: {normalized:?}");
    let args = expect_planned_call(&normalized[0], "config_basic", "read_field");
    assert_eq!(args.get("field_path").and_then(Value::as_str), Some("name"));
}

#[test]
fn structured_multi_field_contract_rewrites_broad_read_to_read_fields() {
    let root = TempDirGuard::new("structured_multi_field_plan");
    let config_dir = root.path.join("configs");
    fs::create_dir_all(&config_dir).expect("create config dir");
    let config = config_dir.join("app_config.toml");
    fs::write(
        &config,
        r#"[app]
name = "RustClaw NL Fixture"

[paths]
docs_dir = "docs"
logs_dir = "logs"
db_path = "data/test_contract.sqlite"
"#,
    )
    .expect("write config");
    let config_path = config.display().to_string();
    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = config_path.clone();
    route.resolved_intent =
        "Return paths.logs_dir and paths.db_path from app_config.toml.".to_string();
    assert_eq!(
            structured_field_selectors(
                &route,
                "scripts/nl_tests/fixtures/device_local/configs/app_config.toml 의 paths.logs_dir 와 paths.db_path 값만 알려줘.",
                true,
                None,
                Some(&config_path),
            ),
            vec!["paths.logs_dir".to_string(), "paths.db_path".to_string()]
        );
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": config_path,
                "mode": "head",
                "n": 120,
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
    ];

    let normalized = normalize_planned_actions(
            &state,
            Some(&route),
            &LoopState::new(1),
            "scripts/nl_tests/fixtures/device_local/configs/app_config.toml 의 paths.logs_dir 와 paths.db_path 값만 알려줘.",
            None,
            actions,
        );

    assert_eq!(normalized.len(), 1, "normalized actions: {normalized:?}");
    let args = expect_planned_call(&normalized[0], "config_basic", "read_fields");
    let field_paths = args
        .get("field_paths")
        .and_then(Value::as_array)
        .expect("field_paths");
    assert_eq!(
        field_paths,
        &vec![json!("paths.logs_dir"), json!("paths.db_path")]
    );
}

#[test]
fn structured_multi_field_rewrite_ignores_background_filename_tokens() {
    let root = TempDirGuard::new("structured_multi_field_background_paths");
    let schema_dir = root.path.join("prompts/schemas");
    fs::create_dir_all(&schema_dir).expect("create schema dir");
    let schema = schema_dir.join("intent_normalizer.schema.json");
    fs::write(
        &schema,
        r#"{"type":"object","properties":{"kind":{"type":"string"}}}"#,
    )
    .expect("write schema");
    let schema_path = schema.display().to_string();
    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = schema_path.clone();
    route.resolved_intent =
        "List schema files, find the largest, and summarize its purpose.".to_string();
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "read_text_range",
            "path": schema_path,
            "mode": "head",
            "n": 50,
        }),
    }];

    let normalized = normalize_planned_actions(
            &state,
            Some(&route),
            &LoopState::new(1),
            "列出 prompts/schemas 下的 json 文件，找最大的并总结它描述什么对象。",
            Some(
                "STABLE_FACTS: 甲文件指向 docs/release_checklist.md，另一个文件是 docs/service_notes.md",
            ),
            actions,
        );

    assert!(
        normalized
            .iter()
            .any(|action| planned_call_is(action, "fs_basic", "read_text_range")),
        "normalized actions: {normalized:?}"
    );
    assert!(
        normalized.iter().all(
            |action| !planned_call_is(action, "config_basic", "read_fields")
                && !planned_call_is(action, "config_basic", "validate")
        ),
        "normalized actions: {normalized:?}"
    );
}

#[test]
fn structured_multi_field_contract_rewrites_key_listing_to_read_fields() {
    let root = TempDirGuard::new("structured_multi_field_list_keys_plan");
    let config_dir = root.path.join("configs");
    fs::create_dir_all(&config_dir).expect("create config dir");
    let config = config_dir.join("app_config.toml");
    fs::write(
        &config,
        r#"[paths]
logs_dir = "logs"
db_path = "data/test_contract.sqlite"
"#,
    )
    .expect("write config");
    let config_path = config.display().to_string();
    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = config_path.clone();
    route.resolved_intent = format!("Return paths.logs_dir and paths.db_path from {config_path}.");
    let actions = vec![AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args: json!({
            "action": "list_keys",
            "path": config_path,
            "max_keys": 1000,
        }),
    }];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "Return paths.logs_dir and paths.db_path from configs/app_config.toml.",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 1, "normalized actions: {normalized:?}");
    let args = expect_planned_call(&normalized[0], "config_basic", "read_fields");
    let field_paths = args
        .get("field_paths")
        .and_then(Value::as_array)
        .expect("field_paths");
    assert_eq!(
        field_paths,
        &vec![json!("paths.logs_dir"), json!("paths.db_path")]
    );
}

#[test]
fn structured_identity_scalar_contract_rewrites_broad_read_to_read_field() {
    let root = TempDirGuard::new("structured_identity_field_plan");
    let registry = root.path.join("skills_registry.toml");
    fs::write(
        &registry,
        r#"[[skills]]
name = "fs_basic"
group = "filesystem"
planner_kind = "tool"

[[skills]]
name = "archive_basic"
group = "archive"
planner_kind = "tool"
"#,
    )
    .expect("write registry");
    let registry_path = registry.display().to_string();
    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = registry_path.clone();
    route.resolved_intent =
        "Read skills_registry.toml and return the group value for archive_basic.".to_string();
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "read_text_range",
            "path": registry_path,
            "mode": "head",
            "n": 120,
        }),
    }];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "configs/skills_registry.toml で archive_basic の group だけ答えて。",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 1);
    let args = expect_planned_call(&normalized[0], "config_basic", "read_field");
    assert_eq!(
        args.get("field_path").and_then(Value::as_str),
        Some("archive_basic.group")
    );
}

#[test]
fn structured_identity_presence_contract_rewrites_stat_to_read_field() {
    let root = TempDirGuard::new("structured_identity_presence_plan");
    let registry = root.path.join("skills_registry.toml");
    fs::write(
        &registry,
        r#"[[skills]]
name = "fs_basic"
group = "filesystem"
planner_kind = "tool"

[[skills]]
name = "archive_basic"
group = "archive"
planner_kind = "tool"
"#,
    )
    .expect("write registry");
    let registry_path = registry.display().to_string();
    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = registry_path.clone();
    route.route_reason = "structured_identifier_presence_requires_content_evidence".to_string();
    route.resolved_intent =
        "Read skills_registry.toml and answer whether fs_basic is registered.".to_string();
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "stat_paths",
            "paths": [registry_path],
            "include_missing": true,
        }),
    }];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "Read skills_registry.toml and answer whether fs_basic is registered.",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 1);
    let args = expect_planned_call(&normalized[0], "config_basic", "read_field");
    assert_eq!(
        args.get("field_path").and_then(Value::as_str),
        Some("fs_basic.name")
    );
}

#[test]
fn structured_identity_presence_contract_rewrites_validate_to_read_field() {
    let root = TempDirGuard::new("structured_identity_presence_validate_plan");
    let registry = root.path.join("skills_registry.toml");
    fs::write(
        &registry,
        r#"[[skills]]
name = "fs_basic"
group = "filesystem"
planner_kind = "tool"
"#,
    )
    .expect("write registry");
    let registry_path = registry.display().to_string();
    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = registry_path.clone();
    route.route_reason = "structured_identifier_presence_requires_content_evidence".to_string();
    route.resolved_intent =
        "Read skills_registry.toml and answer whether fs_basic is registered.".to_string();
    let actions = vec![AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args: json!({
            "action": "validate",
            "path": registry_path,
            "format": "toml",
        }),
    }];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "Read skills_registry.toml and answer whether fs_basic is registered.",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 1);
    let args = expect_planned_call(&normalized[0], "config_basic", "read_field");
    assert_eq!(
        args.get("field_path").and_then(Value::as_str),
        Some("fs_basic.name")
    );
}

#[test]
fn structured_identity_presence_deterministic_plan_reads_identity_field() {
    let root = TempDirGuard::new("structured_identity_presence_deterministic_plan");
    let registry = root.path.join("skills_registry.toml");
    fs::write(
        &registry,
        r#"[[skills]]
name = "fs_basic"
enabled = true
group = "filesystem"
planner_kind = "tool"

[[skills]]
name = "archive_basic"
enabled = true
group = "archive"
planner_kind = "tool"
"#,
    )
    .expect("write registry");
    let registry_path = registry.display().to_string();
    let mut state = test_state_with_enabled_skills(&["config_basic", "fs_basic"]);
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = registry_path.clone();
    route.route_reason = "structured_identifier_presence_requires_content_evidence".to_string();
    route.resolved_intent =
        "Read skills_registry.toml and answer whether fs_basic is registered.".to_string();
    let request = "Read skills_registry.toml and answer whether fs_basic is registered.";

    let plan = super::super::scalar_content_auto_locator_deterministic_plan_result(
        &state,
        request,
        Some(&route),
        &LoopState::new(1),
        request,
        Some(request),
        Some(registry_path.as_str()),
    )
    .expect("structured identity presence should bypass broad file reads");

    assert_eq!(plan.steps.len(), 1);
    let first = plan.steps[0]
        .to_agent_action()
        .expect("first step should be an action");
    let args = expect_planned_call(&first, "config_basic", "read_field");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(registry_path.as_str())
    );
    assert_eq!(
        args.get("field_path").and_then(Value::as_str),
        Some("fs_basic.name")
    );
}

#[test]
fn content_excerpt_structured_scalar_field_deterministic_plan_uses_read_field() {
    let root = TempDirGuard::new("content_excerpt_structured_scalar_field_plan");
    let registry = root.path.join("skills_registry.toml");
    fs::write(
        &registry,
        r#"[[skills]]
name = "run_cmd"
group = "system"
planner_kind = "tool"

[[skills]]
name = "fs_basic"
group = "filesystem"
planner_kind = "tool"
"#,
    )
    .expect("write registry");
    let registry_path = registry.display().to_string();
    let mut state = test_state_with_enabled_skills(&["config_basic", "fs_basic"]);
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = registry_path.clone();
    route.resolved_intent =
        "Locate the run_cmd configuration in skills_registry.toml and report planner_kind."
            .to_string();
    let request =
        "在 configs/skills_registry.toml 里找到 run_cmd 相关配置位置，并告诉我它的 planner_kind 是什么";

    let plan = structured_scalar_field_auto_locator_deterministic_plan_result(
        &state,
        request,
        Some(&route),
        &LoopState::new(1),
        request,
        Some(request),
        Some(registry_path.as_str()),
    )
    .expect("structured scalar field should bypass broad content reads");

    assert_eq!(plan.steps.len(), 3);
    let first = plan.steps[0]
        .to_agent_action()
        .expect("first step should be an action");
    let args = expect_planned_call(&first, "config_basic", "read_field");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(registry_path.as_str())
    );
    assert_eq!(
        args.get("field_path").and_then(Value::as_str),
        Some("run_cmd.planner_kind")
    );
    assert!(matches!(
        plan.steps[1].to_agent_action().as_ref(),
        Some(AgentAction::SynthesizeAnswer { evidence_refs })
            if evidence_refs == &vec!["last_output".to_string()]
    ));
    assert!(matches!(
        plan.steps[2].to_agent_action().as_ref(),
        Some(AgentAction::Respond { content }) if content == "{{last_output}}"
    ));
}

#[test]
fn rustclaw_config_validation_without_profile_keeps_validate_action() {
    let mut route = base_route_result();
    route.resolved_intent =
        "Legacy risk/problem wording in route text must not trigger runtime rewrites.".to_string();
    route.output_contract.locator_hint = "configs/config.toml".to_string();
    let actions = vec![AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args: json!({
            "action": "validate",
            "path": "configs/config.toml",
            "format": "toml",
        }),
    }];

    let rewritten = rewrite_rustclaw_config_validation_to_guard(Some(&route), None, actions);

    expect_planned_call(&rewritten[0], "config_basic", "validate");
}

#[test]
fn config_validate_capability_ref_without_semantic_kind_keeps_validate_action() {
    let mut route = base_route_result();
    route.route_reason = "capability_ref=config.validate".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();
    let actions = vec![AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args: json!({
            "action": "validate",
            "path": "configs/config.toml",
            "format": "toml",
        }),
    }];

    let rewritten = rewrite_rustclaw_config_validation_to_guard(Some(&route), None, actions);

    expect_planned_call(&rewritten[0], "config_basic", "validate");
}

#[test]
fn config_guard_capability_ref_allows_direct_observed_finalize_without_semantic_kind() {
    let mut route = base_route_result();
    route.route_reason = "capability_ref=config.guard_after_change".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();

    assert!(action_supports_structured_direct_observed_finalize(
        Some(&route),
        "config_edit",
        &json!({
            "action": "guard_config",
            "path": "configs/config.toml",
        }),
    ));
}

#[test]
fn archive_capability_ref_uses_runtime_owned_observed_finalizer_without_semantic_kind() {
    let mut route = base_route_result();
    route.route_reason = "capability_ref=archive.list".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;

    assert!(route_uses_runtime_owned_observed_finalizer(&route));
}

#[test]
fn rustclaw_config_guard_profile_without_locator_keeps_validate_action() {
    let mut route = base_route_result();
    route.output_contract.locator_hint.clear();
    let actions = vec![AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args: json!({
            "action": "validate",
            "format": "toml",
            "validation_profile": "rustclaw_semantic_guard",
        }),
    }];

    let rewritten = rewrite_rustclaw_config_validation_to_guard(Some(&route), None, actions);

    expect_planned_call(&rewritten[0], "config_basic", "validate");
}

#[test]
fn archive_basic_pack_output_alias_normalizes_to_archive() {
    let actions = vec![AgentAction::CallSkill {
        skill: "archive_basic".to_string(),
        args: json!({
            "action": "pack",
            "source": "scripts/skill_calls",
            "output": "tmp/nl_archive_case.zip",
        }),
    }];

    let normalized = normalize_archive_basic_schema_aliases(None, actions);

    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "archive_basic");
            assert_eq!(
                args.get("archive").and_then(Value::as_str),
                Some("tmp/nl_archive_case.zip")
            );
            assert_eq!(args.get("format").and_then(Value::as_str), Some("zip"));
            assert!(args.get("output").is_none());
        }
        other => panic!("expected archive_basic action, got {other:?}"),
    }
}

#[test]
fn archive_basic_list_path_alias_normalizes_to_archive_contract() {
    let actions = vec![AgentAction::CallSkill {
        skill: "archive_basic".to_string(),
        args: json!({
            "action": "list",
            "path": "/tmp/rustclaw_archive_nl_case/sample.tgz",
        }),
    }];

    let normalized = normalize_archive_basic_schema_aliases(None, actions);

    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "archive_basic");
            assert_eq!(
                args.get("archive").and_then(Value::as_str),
                Some("/tmp/rustclaw_archive_nl_case/sample.tgz")
            );
            assert!(args.get("path").is_none());
        }
        other => panic!("expected archive_basic action, got {other:?}"),
    }
}

#[test]
fn archive_basic_read_action_preserves_member_contract() {
    let archive = "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip";
    let actions = vec![AgentAction::CallSkill {
        skill: "archive_basic".to_string(),
        args: json!({
            "action": "read",
            "path": archive,
            "entry": "notes.txt",
        }),
    }];

    let normalized = normalize_archive_basic_schema_aliases(None, actions);

    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "archive_basic");
            assert_eq!(args.get("action").and_then(Value::as_str), Some("read"));
            assert_eq!(args.get("archive").and_then(Value::as_str), Some(archive));
            assert_eq!(
                args.get("member").and_then(Value::as_str),
                Some("notes.txt")
            );
        }
        other => panic!("expected archive_basic action, got {other:?}"),
    }
}

#[test]
fn archive_basic_short_list_archive_uses_active_bound_target() {
    let bound_target = "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip";
    let actions = vec![AgentAction::CallSkill {
        skill: "archive_basic".to_string(),
        args: json!({
            "action": "list",
            "archive": "test_bundle.zip",
        }),
    }];
    let plan_context = format!(
            "### ACTIVE_EXECUTION_ANCHOR\nfollowup_op_kind: Read\nfollowup_bound_target: {bound_target}\nobserved_bound_target: {bound_target}"
        );

    let rewritten =
        rewrite_archive_basic_short_archive_to_active_bound_target(Some(&plan_context), actions);

    match &rewritten[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "archive_basic");
            assert_eq!(
                args.get("archive").and_then(Value::as_str),
                Some(bound_target)
            );
        }
        other => panic!("expected archive_basic action, got {other:?}"),
    }
}

#[test]
fn package_manager_detect_uses_deterministic_skill_plan() {
    let state = test_state_with_enabled_skills(&["package_manager"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.resolved_intent = "capability_ref=package.detect_manager".to_string();
    let loop_state = LoopState::new(1);

    let plan = package_manager_detect_deterministic_plan_result(
        &state,
        "detect package manager",
        Some(&route),
        &loop_state,
        Some("/workspace/UI"),
    )
    .expect("package manager detection should use deterministic plan");

    assert_eq!(plan.steps.len(), 3);
    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "package_manager", "detect");
    assert_eq!(args.get("action").and_then(Value::as_str), Some("detect"));
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some("/workspace/UI")
    );
}

#[test]
fn package_docker_probe_uses_structured_readonly_skills_for_service_status_route() {
    let state = test_state_with_enabled_skills(&["package_manager", "docker_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    route.route_reason =
        "machine_plan: capability_ref=package.detect_manager capability_ref=docker.version capability_ref=docker.list_containers"
            .to_string();
    let loop_state = LoopState::new(1);

    let plan = package_docker_readonly_probe_deterministic_plan_result(
        &state,
        "package and docker readonly probe",
        Some(&route),
        &loop_state,
    )
    .expect("package/docker probe should use structured readonly skills");

    assert_eq!(plan.steps.len(), 5);
    let package_action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&package_action, "package_manager", "detect");
    assert_eq!(args.get("action").and_then(Value::as_str), Some("detect"));
    let docker_version_action = plan.steps[1].to_agent_action().expect("agent action");
    let args = expect_planned_call(&docker_version_action, "docker_basic", "version");
    assert_eq!(args.get("action").and_then(Value::as_str), Some("version"));
    let docker_ps_action = plan.steps[2].to_agent_action().expect("agent action");
    let args = expect_planned_call(&docker_ps_action, "docker_basic", "ps");
    assert_eq!(args.get("action").and_then(Value::as_str), Some("ps"));
    assert_eq!(plan.steps[3].action_type, "synthesize_answer");
    assert_eq!(plan.steps[4].action_type, "respond");
}

#[test]
fn package_docker_probe_overrides_content_excerpt_auto_locator_route() {
    let state = test_state_with_enabled_skills(&["package_manager", "docker_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/home/guagua/rustclaw/docker".to_string();
    route.route_reason =
        "machine_plan: capability_ref=package.detect_manager capability_ref=docker.list_containers capability_ref=docker.version read_only_no_mutation_no_retry"
            .to_string();
    let loop_state = LoopState::new(1);

    let plan = package_docker_readonly_probe_deterministic_plan_result(
        &state,
        "package and docker readonly probe",
        Some(&route),
        &loop_state,
    )
    .expect("package/docker probe should use structured skills despite locator fallback");

    assert_eq!(plan.steps.len(), 5);
    let package_action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&package_action, "package_manager", "detect");
    assert_eq!(args.get("action").and_then(Value::as_str), Some("detect"));
    let docker_version_action = plan.steps[1].to_agent_action().expect("agent action");
    let args = expect_planned_call(&docker_version_action, "docker_basic", "version");
    assert_eq!(args.get("action").and_then(Value::as_str), Some("version"));
    let docker_ps_action = plan.steps[2].to_agent_action().expect("agent action");
    let args = expect_planned_call(&docker_ps_action, "docker_basic", "ps");
    assert_eq!(args.get("action").and_then(Value::as_str), Some("ps"));
}

#[test]
fn package_docker_probe_requires_exact_capability_tokens() {
    let state = test_state_with_enabled_skills(&["package_manager", "docker_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.route_reason =
        "machine_plan: capability_ref=package.detect_manager capability_ref=docker.version_extra"
            .to_string();
    let loop_state = LoopState::new(1);

    assert!(package_docker_readonly_probe_deterministic_plan_result(
        &state,
        "package and docker readonly probe",
        Some(&route),
        &loop_state,
    )
    .is_none());
}

#[test]
fn package_docker_probe_ignores_legacy_machine_tokens_without_capability_ref() {
    let state = test_state_with_enabled_skills(&["package_manager", "docker_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = OutputSemanticKind::DockerContainerLifecycle;
    route.route_reason =
        "machine_plan: package_manager_detection docker_container_lifecycle docker_ps".to_string();
    let loop_state = LoopState::new(1);

    assert!(package_docker_readonly_probe_deterministic_plan_result(
        &state,
        "package and docker readonly probe",
        Some(&route),
        &loop_state,
    )
    .is_none());
}

#[test]
fn contract_hint_preferred_run_cmd_uses_machine_hint_not_request_words() {
    let state = test_state_with_enabled_skills(&["run_cmd", "package_manager"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.resolved_intent = "capability_ref=package.detect_manager".to_string();
    route.output_contract.locator_kind = OutputLocatorKind::None;
    let request =
            "arbitrary multilingual surface\n[CONTRACT_TEST_HINT]\npreferred_action_ref=run_cmd\n[/CONTRACT_TEST_HINT]";

    let plan = contract_hint_preferred_action_deterministic_plan_result(
        &state,
        "detect package manager",
        Some(&route),
        &LoopState::new(1),
        request,
        None,
    )
    .expect("machine hint should select run_cmd");

    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].skill, "run_cmd");
    assert!(plan.steps[0]
        .args
        .get("command")
        .and_then(Value::as_str)
        .is_some());
}

#[test]
fn contract_hint_preferred_run_cmd_sqlite_without_database_capability_is_rejected() {
    let state = test_state_with_enabled_skills(&["run_cmd", "db_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = OutputSemanticKind::SqliteDatabaseKindJudgment;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite".to_string();
    let request = "[CONTRACT_TEST_HINT]\npreferred_action_ref=run_cmd\n[/CONTRACT_TEST_HINT]";

    let plan = contract_hint_preferred_action_deterministic_plan_result(
        &state,
        "inspect sqlite database kind",
        Some(&route),
        &LoopState::new(1),
        request,
        None,
    );

    assert!(
        plan.is_none(),
        "sqlite semantic marker alone must not authorize a run_cmd database probe"
    );
}

#[test]
fn contract_hint_preferred_db_basic_does_not_claim_structured_keys_config_file() {
    let root = TempDirGuard::new("contract_hint_structured_keys_db_basic");
    let config_path = root.path.join("config.toml");
    fs::write(&config_path, "alpha = 1\n[beta]\nvalue = 2\n").expect("write config");
    let config_path = config_path.display().to_string();
    let state = test_state_with_enabled_skills(&["config_basic", "db_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = OutputSemanticKind::StructuredKeys;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = config_path.clone();
    let request = "[CONTRACT_TEST_HINT]\npreferred_action_ref=db_basic\n[/CONTRACT_TEST_HINT]";

    assert!(contract_hint_preferred_action_deterministic_plan_result(
        &state,
        "list structured keys",
        Some(&route),
        &LoopState::new(1),
        request,
        Some(&config_path),
    )
    .is_none());

    let plan = structured_keys_deterministic_plan_result(
        &state,
        "list structured keys",
        "list structured keys",
        Some(&route),
        &LoopState::new(1),
        Some(&config_path),
    )
    .expect("structured keys should fall back to config_basic list_keys");
    assert_eq!(plan.steps.len(), 1);
    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "config_basic", "list_keys");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(config_path.as_str())
    );
}

#[test]
fn contract_hint_workspace_summary_list_dir_prefers_text_excerpt_evidence() {
    let root = TempDirGuard::new("contract_hint_workspace_summary_list_dir");
    fs::write(
        root.path.join("README.md"),
        "# Fixture\n\nThis directory contains local test fixtures.",
    )
    .expect("write README");
    let root_path = root.path.display().to_string();
    let state = test_state_with_enabled_skills(&["fs_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = OutputSemanticKind::WorkspaceProjectSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_path.clone();
    let request =
        "[CONTRACT_TEST_HINT]\npreferred_action_ref=fs_basic.list_dir\n[/CONTRACT_TEST_HINT]";

    let plan = contract_hint_preferred_action_deterministic_plan_result(
        &state,
        "summarize workspace",
        Some(&route),
        &LoopState::new(1),
        request,
        Some(&root_path),
    )
    .expect("workspace summary should use readable text evidence");

    assert_eq!(plan.steps.len(), 1);
    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "fs_basic", "read_text_range");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(root.path.join("README.md").display().to_string().as_str())
    );
}

#[test]
fn contract_hint_workspace_summary_git_basic_prefers_text_excerpt_evidence() {
    let root = TempDirGuard::new("contract_hint_workspace_summary_git_basic");
    fs::write(
        root.path.join("README.md"),
        "# Fixture\n\nThis directory contains local test fixtures.",
    )
    .expect("write README");
    let root_path = root.path.display().to_string();
    let state = test_state_with_enabled_skills(&["fs_basic", "git_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = OutputSemanticKind::WorkspaceProjectSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_path.clone();
    let request = "[CONTRACT_TEST_HINT]\npreferred_action_ref=git_basic\n[/CONTRACT_TEST_HINT]";

    let plan = contract_hint_preferred_action_deterministic_plan_result(
        &state,
        "summarize workspace",
        Some(&route),
        &LoopState::new(1),
        request,
        Some(&root_path),
    )
    .expect("workspace summary should use readable text evidence");

    assert_eq!(plan.steps.len(), 1);
    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "fs_basic", "read_text_range");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(root.path.join("README.md").display().to_string().as_str())
    );
}

#[test]
fn contract_hint_generic_path_content_stat_paths_prefers_text_excerpt_evidence() {
    let root = TempDirGuard::new("contract_hint_generic_path_content_stat_paths");
    let doc_path = root.path.join("release_checklist.md");
    fs::write(
        &doc_path,
        "# Release Checklist\n\n- Verify config loading\n- Check recent logs\n",
    )
    .expect("write doc");
    let doc_path = doc_path.display().to_string();
    let state = test_state_with_enabled_skills(&["fs_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = doc_path.clone();
    let request =
        "[CONTRACT_TEST_HINT]\npreferred_action_ref=fs_basic.stat_paths\n[/CONTRACT_TEST_HINT]";

    let plan = contract_hint_preferred_action_deterministic_plan_result(
        &state,
        "summarize file",
        Some(&route),
        &LoopState::new(1),
        request,
        Some(&doc_path),
    )
    .expect("generic file summary should use readable text evidence");

    assert_eq!(plan.steps.len(), 1);
    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "fs_basic", "read_text_range");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(doc_path.as_str())
    );
}

#[test]
fn contract_hint_preferred_fs_stat_paths_uses_locator_contract() {
    let state = test_state_with_enabled_skills(&["fs_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/package.json".to_string();
    let request =
        "[CONTRACT_TEST_HINT]\npreferred_action_ref=fs_basic.stat_paths\n[/CONTRACT_TEST_HINT]";

    let plan = contract_hint_preferred_action_deterministic_plan_result(
        &state,
        "return path",
        Some(&route),
        &LoopState::new(1),
        request,
        None,
    )
    .expect("machine hint should select fs_basic.stat_paths");

    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].skill, "fs_basic");
    assert_eq!(
        plan.steps[0].args.get("action").and_then(Value::as_str),
        Some("stat_paths")
    );
}

#[test]
fn contract_hint_scalar_equality_without_locator_falls_back_to_git_branch() {
    let state = test_state_with_enabled_skills(&["fs_basic", "git_basic", "run_cmd"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = OutputSemanticKind::RecentScalarEqualityCheck;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    let request = "[CONTRACT_TEST_HINT]\nsemantic_kind=recent_scalar_equality_check\ncandidate_wrong_action_ref=db_basic\n[/CONTRACT_TEST_HINT]";

    let plan = contract_hint_preferred_action_deterministic_plan_result(
        &state,
        "check scalar equality",
        Some(&route),
        &LoopState::new(1),
        request,
        None,
    )
    .expect("matrix fallback should select git_basic when no path locator exists");

    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].skill, "git_basic");
    assert_eq!(
        plan.steps[0].args.get("action").and_then(Value::as_str),
        Some("current_branch")
    );
}

#[test]
fn contract_hint_matrix_preferred_workspace_summary_reads_text_evidence() {
    let mut state = test_state_with_enabled_skills(&["fs_basic"]);
    let root = TempDirGuard::new("contract_hint_workspace_summary");
    let fixture_dir = root.path.join("fixture_project");
    fs::create_dir_all(&fixture_dir).expect("create fixture dir");
    fs::write(
        fixture_dir.join("README.md"),
        "# Fixture Project\n\nA small local project used by contract tests.\n",
    )
    .expect("write readme");
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.semantic_kind = OutputSemanticKind::WorkspaceProjectSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "fixture_project".to_string();
    let request =
        "[CONTRACT_TEST_HINT]\nsemantic_kind=workspace_project_summary\n[/CONTRACT_TEST_HINT]";

    let plan = contract_hint_preferred_action_deterministic_plan_result(
        &state,
        "summarize project",
        Some(&route),
        &LoopState::new(1),
        request,
        None,
    )
    .expect("matrix preferred action should select readable text evidence");

    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].skill, "fs_basic");
    assert_eq!(
        plan.steps[0].args.get("action").and_then(Value::as_str),
        Some("read_text_range")
    );
    assert!(plan.steps[0]
        .args
        .get("path")
        .and_then(Value::as_str)
        .is_some_and(|path| path.ends_with("fixture_project/README.md")));
}

#[test]
fn contract_hint_preferred_docker_logs_does_not_use_legacy_semantic_fast_path() {
    let state = test_state_with_enabled_skills(&["docker_basic", "run_cmd"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.resolved_intent = "capability_ref=docker.read_logs".to_string();
    let request = "[CONTRACT_TEST_HINT]\npreferred_action_ref=docker_basic\n[/CONTRACT_TEST_HINT]";

    let plan = contract_hint_preferred_action_deterministic_plan_result(
        &state,
        "inspect docker logs",
        Some(&route),
        &LoopState::new(1),
        request,
        None,
    );

    assert!(plan.is_none());
}

#[test]
fn preferred_docker_basic_uses_capability_ref_with_semantic_none() {
    let state = test_state_with_enabled_skills(&["docker_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.route_reason = "capability_ref=docker.list_images".to_string();
    let preferred = crate::contract_matrix::ActionRef {
        skill: "docker_basic".to_string(),
        action: None,
    };

    let action =
        preferred_structured_action_for_contract_hint(&state, &route, &preferred, None, "")
            .expect("docker_basic capability ref should choose structured action");

    match action {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "docker_basic");
            assert_eq!(args.get("action").and_then(Value::as_str), Some("images"));
        }
        other => panic!("expected docker_basic action, got {other:?}"),
    }
}

#[test]
fn preferred_docker_basic_ignores_legacy_semantic_without_capability_ref() {
    let state = test_state_with_enabled_skills(&["docker_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = OutputSemanticKind::DockerImages;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    let preferred = crate::contract_matrix::ActionRef {
        skill: "docker_basic".to_string(),
        action: None,
    };

    let action =
        preferred_structured_action_for_contract_hint(&state, &route, &preferred, None, "")
            .expect("docker_basic remains available, but legacy marker must not choose images");

    match action {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "docker_basic");
            assert_eq!(args.get("action").and_then(Value::as_str), Some("ps"));
        }
        other => panic!("expected docker_basic action, got {other:?}"),
    }
}

#[test]
fn preferred_archive_basic_uses_capability_ref_with_semantic_none() {
    let state = test_state_with_enabled_skills(&["archive_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip|notes.txt".to_string();
    route.route_reason = "capability_ref=archive.read".to_string();
    let preferred = crate::contract_matrix::ActionRef {
        skill: "archive_basic".to_string(),
        action: None,
    };

    let action = preferred_structured_action_for_contract_hint(
        &state,
        &route,
        &preferred,
        Some("scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip"),
        "Read member notes.txt from scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip",
    )
    .expect("archive capability ref should choose structured read action");

    match action {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "archive_basic");
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
        other => panic!("expected archive_basic action, got {other:?}"),
    }
}

#[test]
fn preferred_config_basic_uses_capability_ref_with_semantic_none() {
    let state = test_state_with_enabled_skills(&["config_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();
    route.route_reason = "capability_ref=config.guard_rustclaw_config".to_string();
    let preferred = crate::contract_matrix::ActionRef {
        skill: "config_basic".to_string(),
        action: None,
    };

    let action =
        preferred_structured_action_for_contract_hint(&state, &route, &preferred, None, "")
            .expect("config_basic capability ref should choose guard action");

    match action {
        AgentAction::CallTool { tool, args } => {
            assert_eq!(tool, "config_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("guard_rustclaw_config")
            );
            assert_eq!(
                args.get("path").and_then(Value::as_str),
                Some("configs/config.toml")
            );
        }
        other => panic!("expected config_basic action, got {other:?}"),
    }
}

#[test]
fn preferred_config_edit_uses_capability_ref_with_semantic_none() {
    let state = test_state_with_enabled_skills(&["config_edit"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();
    route.route_reason = "capability_ref=config.validate_after_change".to_string();
    let preferred = crate::contract_matrix::ActionRef {
        skill: "config_edit".to_string(),
        action: None,
    };

    let action =
        preferred_structured_action_for_contract_hint(&state, &route, &preferred, None, "")
            .expect("config_edit capability ref should choose validate action");

    match action {
        AgentAction::CallTool { tool, args } => {
            assert_eq!(tool, "config_edit");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("validate_config")
            );
            assert_eq!(
                args.get("path").and_then(Value::as_str),
                Some("configs/config.toml")
            );
        }
        other => panic!("expected config_edit action, got {other:?}"),
    }
}

#[test]
fn contract_hint_preferred_run_cmd_uses_docker_capability_ref_with_semantic_none() {
    let state = test_state_with_enabled_skills(&["run_cmd"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.resolved_intent = "capability_ref=docker.list_images".to_string();
    let request =
        "arbitrary multilingual surface\n[CONTRACT_TEST_HINT]\npreferred_action_ref=run_cmd\n[/CONTRACT_TEST_HINT]";

    let plan = contract_hint_preferred_action_deterministic_plan_result(
        &state,
        "list docker images",
        Some(&route),
        &LoopState::new(1),
        request,
        None,
    )
    .expect("machine hint and capability ref should select run_cmd");

    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].skill, "run_cmd");
    assert!(plan.steps[0]
        .args
        .get("command")
        .and_then(Value::as_str)
        .is_some_and(|command| command.contains("docker images")));
}

#[test]
fn contract_hint_preferred_run_cmd_ignores_legacy_docker_semantic_without_capability_ref() {
    let state = test_state_with_enabled_skills(&["run_cmd"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = OutputSemanticKind::DockerImages;
    route.output_contract.locator_kind = OutputLocatorKind::None;

    assert!(preferred_run_cmd_for_contract_hint(&state, &route, None).is_none());
}

#[test]
fn contract_hint_matrix_existence_summary_reads_stat_and_content_from_route_context() {
    let mut state = test_state_with_enabled_skills(&["fs_basic"]);
    let root = TempDirGuard::new("contract_hint_existence_summary");
    let fixture = root.path.join("package.json");
    fs::write(
        &fixture,
        r#"{"name":"rustclaw-nl-fixture","description":"local fixture package"}"#,
    )
    .expect("write fixture");
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = base_route_result();
    route.route_reason = "structured_contract_hint_fast_path; contract_hint_fast_path".into();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPathSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "package.json".to_string();

    let plan = contract_hint_preferred_action_deterministic_plan_result(
        &state,
        "describe package",
        Some(&route),
        &LoopState::new(1),
        "sanitized user request without machine hint block",
        None,
    )
    .expect("route-level contract hint should select deterministic two-step plan");

    assert_eq!(plan.steps.len(), 4);
    assert_eq!(plan.steps[0].skill, "fs_basic");
    assert_eq!(
        plan.steps[0].args.get("action").and_then(Value::as_str),
        Some("stat_paths")
    );
    assert_eq!(plan.steps[1].skill, "fs_basic");
    assert_eq!(
        plan.steps[1].args.get("action").and_then(Value::as_str),
        Some("read_text_range")
    );
    assert!(plan.steps[1]
        .args
        .get("path")
        .and_then(Value::as_str)
        .is_some_and(|path| path.ends_with("package.json")));
}

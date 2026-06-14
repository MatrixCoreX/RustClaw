use super::*;

#[test]
fn structured_keys_contract_uses_deterministic_list_keys_plan() {
    let root = TempDirGuard::new("structured_keys_deterministic_plan");
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

    let state = test_state_with_enabled_skills(&["config_basic"]);
    let loop_state = LoopState::new(2);
    let plan = structured_keys_deterministic_plan_result(
        &state,
        "list structured keys",
        "list structured keys",
        Some(&route),
        &loop_state,
        Some(&config_path),
    )
    .expect("structured keys deterministic plan");

    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.plan_kind, PlanKind::Single);
    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "config_basic", "list_keys");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(config_path.as_str())
    );
}

#[test]
fn structured_keys_plan_ignores_background_field_selectors() {
    let root = TempDirGuard::new("structured_keys_background_field_plan");
    let config_path = root.path.join("config.toml");
    fs::write(
        &config_path,
        "alpha = 1\n[llm]\nselected_vendor = \"minimax\"\n",
    )
    .expect("write config");
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
    route.resolved_intent = "list top-level keys".to_string();

    let state = test_state_with_enabled_skills(&["config_basic"]);
    let plan = structured_keys_deterministic_plan_result(
        &state,
        "BACKGROUND: configs/config.toml llm.selected_vendor is minimax",
        "list top-level keys",
        Some(&route),
        &LoopState::new(1),
        Some(&config_path),
    )
    .expect("structured keys deterministic plan");

    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "config_basic", "list_keys");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(config_path.as_str())
    );
    assert!(args.get("field_path").is_none());
}

#[test]
fn structured_keys_deterministic_plan_preserves_nested_field_path() {
    let root = TempDirGuard::new("structured_keys_nested_field_plan");
    let config_path = root.path.join("package.json");
    fs::write(
        &config_path,
        r#"{"name":"fixture","scripts":{"build":"vite","test":"vitest"}}"#,
    )
    .expect("write package json");
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
    route.resolved_intent = "list keys under scripts".to_string();

    let state = test_state_with_enabled_skills(&["config_basic"]);
    let plan = structured_keys_deterministic_plan_result(
        &state,
        "list keys under scripts",
        "list keys under scripts",
        Some(&route),
        &LoopState::new(1),
        Some(&config_path),
    )
    .expect("structured keys nested field plan");

    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "config_basic", "list_keys");
    assert_eq!(
        args.get("field_path").and_then(Value::as_str),
        Some("scripts")
    );
}

#[test]
fn structured_keys_deterministic_plan_reads_identity_scalar_field_value() {
    let root = TempDirGuard::new("structured_keys_identity_scalar_plan");
    let config_dir = root.path.join("configs");
    fs::create_dir_all(&config_dir).expect("create config dir");
    let config_path = config_dir.join("skills_registry.toml");
    fs::write(
        &config_path,
        r#"[[skills]]
name = "run_cmd"
enabled = true
planner_kind = "tool"

[[skills]]
name = "read_file"
enabled = true
planner_kind = "tool"
"#,
    )
    .expect("write skills registry");
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
    route.resolved_intent =
        "Find the run_cmd related configuration and report planner_kind.".to_string();

    let state = test_state_with_enabled_skills(&["config_basic"]);
    let plan = structured_keys_deterministic_plan_result(
        &state,
        "Find run_cmd planner_kind in configs/skills_registry.toml.",
        "Find run_cmd planner_kind in configs/skills_registry.toml.",
        Some(&route),
        &LoopState::new(1),
        Some(&config_path),
    )
    .expect("structured keys scalar field plan");

    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "config_basic", "read_field");
    assert_eq!(
        args.get("field_path").and_then(Value::as_str),
        Some("run_cmd.planner_kind")
    );
}

#[test]
fn structured_keys_deterministic_plan_resolves_unique_suffix_field_value() {
    let root = TempDirGuard::new("structured_keys_suffix_scalar_plan");
    let config_dir = root.path.join("configs");
    fs::create_dir_all(&config_dir).expect("create config dir");
    let config_path = config_dir.join("config.toml");
    fs::write(
        &config_path,
        r#"[llm]
selected_model = "MiniMax-M2.7"
selected_vendor = "minimax"
"#,
    )
    .expect("write config");
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
    route.resolved_intent = "读取当前选用的大模型 vendor 字段值".to_string();

    let state = test_state_with_enabled_skills(&["config_basic"]);
    let plan = structured_keys_deterministic_plan_result(
        &state,
        "读取 configs/config.toml 里当前选用的大模型 vendor，只输出字段和值",
        "读取 configs/config.toml 里当前选用的大模型 vendor，只输出字段和值",
        Some(&route),
        &LoopState::new(1),
        Some(&config_path),
    )
    .expect("structured keys suffix scalar plan");

    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "config_basic", "read_field");
    assert_eq!(
        args.get("field_path").and_then(Value::as_str),
        Some("llm.selected_vendor")
    );
}

#[test]
fn structured_keys_retry_after_validation_uses_list_keys_plan() {
    use crate::executor::{StepExecutionResult, StepExecutionStatus};

    let root = TempDirGuard::new("structured_keys_retry_plan");
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
    let mut loop_state = LoopState::new(3);
    loop_state.round_no = 2;
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            serde_json::json!({
                "action": "validate_structured",
                "path": config_path,
                "valid": true,
                "root_type": "object"
            })
            .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });

    let state = test_state_with_enabled_skills(&["config_basic"]);
    let plan = structured_keys_deterministic_plan_result(
        &state,
        "list structured keys",
        "list structured keys",
        Some(&route),
        &loop_state,
        Some(&config_path),
    )
    .expect("retry should collect structured keys evidence");

    assert_eq!(plan.plan_kind, PlanKind::Incremental);
    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "config_basic", "list_keys");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(config_path.as_str())
    );
}

#[test]
fn structured_keys_contract_rewrites_multi_field_value_read_to_list_keys() {
    let root = TempDirGuard::new("structured_keys_multi_field_plan");
    let config_path = root.path.join("app_config.toml");
    fs::write(
        &config_path,
        "[app]\nname = \"fixture\"\n[features]\nenabled = true\n[paths]\nlogs_dir = \"logs\"\n",
    )
    .expect("write config");
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
    let actions = vec![AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args: json!({
            "action": "read_fields",
            "path": config_path.clone(),
            "field_paths": ["app", "features", "paths"],
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
fn structured_keys_contract_keeps_explicit_structured_field_read() {
    let root = TempDirGuard::new("structured_keys_field_read_plan");
    let config_path = root.path.join("Cargo.toml");
    fs::write(&config_path, "[package]\nname = \"clawd\"\n").expect("write config");
    let config_path = config_path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::StructuredKeys;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = config_path.clone();
    route.output_contract.delivery_required = false;
    let actions = vec![
        AgentAction::CallTool {
            tool: "config_basic".to_string(),
            args: json!({
                "action": "read_field",
                "path": config_path.clone(),
                "field_path": "package.no_such_key_100_matrix",
                "format": "toml",
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
    ];

    let state = test_state_with_enabled_skills(&["system_basic", "config_basic"]);
    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &LoopState::new(1),
        "read the requested structured field",
        None,
        Some(&config_path),
        actions,
    );

    assert!(normalized.iter().any(|action| {
        matches!(
            action,
            AgentAction::CallTool { tool, args }
                if tool == "config_basic"
                    && args.get("action").and_then(Value::as_str) == Some("read_field")
                    && args.get("path").and_then(Value::as_str) == Some(config_path.as_str())
                    && args.get("field_path").and_then(Value::as_str)
                        == Some("package.no_such_key_100_matrix")
        )
    }));
    assert!(!normalized.iter().any(|action| {
        matches!(
            action,
            AgentAction::CallTool { tool, args }
                if tool == "config_basic"
                    && args.get("action").and_then(Value::as_str) == Some("list_keys")
        )
    }));
}

#[test]
fn strict_structured_keys_contract_rewrites_background_field_read_to_list_keys() {
    let root = TempDirGuard::new("structured_keys_background_field_read_rewrite");
    let config_path = root.path.join("config.toml");
    fs::write(&config_path, "alpha = 1\n[skills]\nvalue = true\n").expect("write config");
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
    let actions = vec![AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args: json!({
            "action": "read_field",
            "path": config_path.clone(),
            "field_path": "skills.value",
            "format": "toml",
        }),
    }];

    let state = test_state_with_enabled_skills(&["system_basic", "config_basic"]);
    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &LoopState::new(1),
        "list top-level keys",
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
    assert!(args.get("field_path").is_none());
}

#[test]
fn generic_scalar_structured_file_plan_rewrites_to_read_field_without_repair_marker() {
    let root = TempDirGuard::new("generic_scalar_structured_field_rewrite");
    let package_path = root.path.join("package.json");
    fs::write(
        &package_path,
        r#"{"dependencies":{"@xdevplatform/xurl":"^1.0.3"}}"#,
    )
    .expect("write root package");
    let ui_dir = root.path.join("UI");
    fs::create_dir_all(&ui_dir).expect("create ui dir");
    fs::write(ui_dir.join("package.json"), r#"{"name":"react-example"}"#)
        .expect("write ui package");
    let package_path = package_path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = "package.json".to_string();
    route.output_contract.delivery_required = false;
    route.route_reason = "llm_semantic_contract_repair:malformed_contract_semantic_repair_needed; scalar_locator_requires_evidence".to_string();
    route.resolved_intent = "读取当前工作区 package.json 文件并提取 name 字段的标量值".to_string();

    let mut state = test_state_with_enabled_skills(&["fs_basic", "config_basic"]);
    state.skill_rt.workspace_root = root.path.clone();
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": package_path.clone(),
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
        "package.json 里的 name 到底是什么，只给值",
        Some(&package_path),
        actions,
    );

    let args = expect_planned_call(&normalized[0], "config_basic", "read_field");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(package_path.as_str())
    );
    assert_eq!(args.get("field_path").and_then(Value::as_str), Some("name"));
}

#[test]
fn structured_scalar_file_plan_uses_contract_field_selector_without_nl_mapping() {
    let root = TempDirGuard::new("structured_scalar_contract_field_selector");
    let package_path = root.path.join("package.json");
    fs::write(&package_path, r#"{"name":"rustclaw","private":true}"#).expect("write package");
    let package_path = package_path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "package.json".to_string();
    route
        .output_contract
        .self_extension
        .structured_field_selector = Some("name".to_string());
    route.resolved_intent = "读取指定结构化文件中的目标字段，仅输出该字段的值".to_string();

    let mut state = test_state_with_enabled_skills(&["fs_basic", "config_basic"]);
    state.skill_rt.workspace_root = root.path.clone();
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": package_path.clone(),
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
        "package.json",
        Some(&package_path),
        actions,
    );

    let args = expect_planned_call(&normalized[0], "config_basic", "read_field");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(package_path.as_str())
    );
    assert_eq!(args.get("field_path").and_then(Value::as_str), Some("name"));
}

#[test]
fn structured_scalar_file_plan_uses_resolved_machine_selector_after_clarify() {
    let root = TempDirGuard::new("structured_scalar_clarify_field_selector");
    let package_path = root.path.join("package.json");
    fs::write(&package_path, r#"{"name":"rustclaw","private":true}"#).expect("write package");
    let package_path = package_path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "package.json".to_string();
    route.resolved_intent =
        "Continue previous structured scalar field request structured_field_selector=name"
            .to_string();

    let mut state = test_state_with_enabled_skills(&["fs_basic", "config_basic"]);
    state.skill_rt.workspace_root = root.path.clone();
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": package_path.clone(),
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
        "package.json",
        Some(&package_path),
        actions,
    );

    let args = expect_planned_call(&normalized[0], "config_basic", "read_field");
    assert_eq!(args.get("field_path").and_then(Value::as_str), Some("name"));
}

#[test]
fn generic_scalar_structured_field_read_stays_bound_to_auto_locator() {
    let root = TempDirGuard::new("generic_scalar_structured_field_auto_locator");
    let package_path = root.path.join("package.json");
    fs::write(
        &package_path,
        r#"{"dependencies":{"@xdevplatform/xurl":"^1.0.3"}}"#,
    )
    .expect("write root package");
    let ui_dir = root.path.join("UI");
    fs::create_dir_all(&ui_dir).expect("create ui dir");
    let ui_package_path = ui_dir.join("package.json");
    fs::write(&ui_package_path, r#"{"name":"react-example"}"#).expect("write ui package");
    let package_path = package_path.display().to_string();
    let ui_package_path = ui_package_path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "package.json".to_string();
    route.output_contract.delivery_required = false;
    route.resolved_intent = "读取当前工作区 package.json 文件并提取 name 字段的标量值".to_string();

    let mut state = test_state_with_enabled_skills(&["fs_basic", "config_basic"]);
    state.skill_rt.workspace_root = root.path.clone();
    let actions = vec![AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args: json!({
            "action": "read_field",
            "path": ui_package_path,
            "field_path": "name",
        }),
    }];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "package.json 里的 name 到底是什么，只给值",
        Some(&package_path),
        actions,
    );

    let args = expect_planned_call(&normalized[0], "config_basic", "read_field");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(package_path.as_str())
    );
    assert_eq!(args.get("field_path").and_then(Value::as_str), Some("name"));
}

#[test]
fn file_names_route_accepts_structured_key_listing_for_structured_document() {
    let root = TempDirGuard::new("file_names_structured_keys_plan");
    let package_path = root.path.join("package.json");
    fs::write(
        &package_path,
        r#"{"scripts":{"build":"vite build","dev":"vite","lint":"eslint ."}}"#,
    )
    .expect("write package");
    let package_path = package_path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::FileNames;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = package_path.clone();
    route.output_contract.delivery_required = false;
    let actions = vec![AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args: json!({
            "action": "list_keys",
            "path": package_path,
            "field_path": "scripts",
            "max_keys": 100,
        }),
    }];

    let state = test_state_with_registry();
    assert!(!actions_use_ad_hoc_command_without_route_preferred_skill(
        &state, &route, &actions
    ));
    assert!(observation_only_plan_can_finalize_from_direct_output(
        &state,
        Some(&route),
        &actions
    ));
    assert!(!should_force_actionable_plan_repair(
        &state,
        Some(&route),
        &LoopState::new(1),
        &actions
    ));
}

#[test]
fn plain_act_read_range_plan_uses_direct_observed_finalizer_without_synthesis() {
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/service_notes.md".to_string();
    route.output_contract.delivery_required = false;
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "read_text_range",
            "path": "/tmp/service_notes.md",
            "mode": "head",
            "n": 10,
        }),
    }];

    let normalized = super::super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(1),
        "read first lines of /tmp/service_notes.md",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 1);
    assert!(matches!(
        &normalized[0],
        AgentAction::CallTool { tool, args }
            if tool == "fs_basic"
                && args.get("action").and_then(Value::as_str) == Some("read_text_range")
    ));
}

#[test]
fn chat_wrapped_read_range_plan_adds_synthesis_terminal_answer() {
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/release_checklist.md".to_string();
    route.output_contract.delivery_required = false;
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "read_text_range",
            "path": "/tmp/release_checklist.md",
            "mode": "head",
            "n": 120,
        }),
    }];

    let normalized = super::super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(1),
        "read /tmp/release_checklist.md and answer from its content",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 3);
    assert!(matches!(
        &normalized[0],
        AgentAction::CallTool { tool, args }
            if tool == "fs_basic"
                && args.get("action").and_then(Value::as_str) == Some("read_text_range")
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
fn registry_prefers_config_basic_for_structured_keys_contract() {
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::StructuredKeys;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "package.json".to_string();
    let preferred = registry_preferred_skill_names_for_route(&test_state_with_registry(), &route);
    assert!(preferred.iter().any(|skill| skill == "config_basic"));
}

#[test]
fn explicit_configured_command_request_rewrites_semantic_substitute_to_run_cmd() {
    let mut state = test_state_with_enabled_skills(&["run_cmd", "system_basic"]);
    state.policy.command_intent.execute_prefixes = vec!["execute ".to_string()];
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    let loop_state = LoopState::new(1);
    let original_request = "execute ls scripts, then summarize the directory";
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "inventory_dir",
                "path": "/workspace/scripts",
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
        &loop_state,
        "list scripts and summarize the directory",
        Some(original_request),
        None,
        actions,
    );

    assert_eq!(normalized.len(), 3);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "run_cmd");
            assert_eq!(
                args.get("request_text").and_then(Value::as_str),
                Some(original_request)
            );
            assert!(args
                .get("cwd")
                .and_then(Value::as_str)
                .is_some_and(|cwd| !cwd.trim().is_empty()));
            assert_eq!(
                args.get("command").and_then(Value::as_str),
                Some("ls scripts")
            );
        }
        other => panic!("expected run_cmd action, got {other:?}"),
    }
    assert!(matches!(
        &normalized[1],
        AgentAction::SynthesizeAnswer { evidence_refs } if evidence_refs == &vec!["last_output".to_string()]
    ));
}

#[test]
fn explicit_command_rewrite_preserves_bounded_configured_execute_prefix() {
    let mut state = test_state_with_enabled_skills(&["run_cmd", "system_basic"]);
    state.policy.command_intent.execute_prefixes = vec!["execute ".to_string()];
    let route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    let loop_state = LoopState::new(1);
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "inventory_dir",
            "path": "/workspace/scripts",
            "names_only": true,
        }),
    }];

    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &loop_state,
        "explain a command",
        Some("execute ls scripts, then explain what it lists"),
        None,
        actions,
    );

    assert!(matches!(
        &normalized[0],
        AgentAction::CallSkill { skill, args }
            if skill == "run_cmd"
                && args.get("command").and_then(Value::as_str) == Some("ls scripts")
    ));
}

#[test]
fn explicit_command_extracts_configured_standalone_command_before_freeform_tail() {
    let mut state = test_state_with_enabled_skills(&["run_cmd", "system_basic"]);
    state.policy.command_intent.execute_prefixes = vec!["run ".to_string()];
    state.policy.command_intent.standalone_commands = vec!["pwd".to_string()];

    assert_eq!(
        super::super::explicit_command_segment(
            &state.policy.command_intent,
            "Run pwd and output only the raw result."
        )
        .as_deref(),
        Some("pwd")
    );
    assert_eq!(
        super::super::explicit_command_segment(
            &state.policy.command_intent,
            "Run cargo test and output only the raw result."
        )
        .as_deref(),
        None
    );
}

#[test]
fn explicit_command_rewrite_preserves_configured_standalone_command_before_freeform_tail() {
    let mut state = test_state_with_enabled_skills(&["run_cmd", "system_basic"]);
    state.policy.command_intent.execute_prefixes = vec!["run ".to_string()];
    state.policy.command_intent.standalone_commands = vec!["pwd".to_string()];
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    let loop_state = LoopState::new(1);
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "inventory_dir",
            "path": "/workspace",
            "names_only": true,
        }),
    }];

    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &loop_state,
        "Get current working directory path",
        Some("Run pwd and output only the raw result."),
        None,
        actions,
    );

    assert!(matches!(
        &normalized[0],
        AgentAction::CallSkill { skill, args }
            if skill == "run_cmd"
                && args.get("command").and_then(Value::as_str) == Some("pwd")
    ));
}

#[test]
fn multi_structured_scalar_observations_append_terminal_synthesis() {
    let state = test_state_with_enabled_skills(&["config_basic"]);
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    let loop_state = LoopState::new(1);
    let actions = vec![
        AgentAction::CallTool {
            tool: "config_basic".to_string(),
            args: json!({
                "action": "read_field",
                "path": "/workspace/package.json",
                "field_path": "name",
            }),
        },
        AgentAction::CallTool {
            tool: "config_basic".to_string(),
            args: json!({
                "action": "read_field",
                "path": "/workspace/crates/clawd/Cargo.toml",
                "field_path": "package.name",
            }),
        },
    ];

    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &loop_state,
        "read two package names and say whether they match",
        None,
        None,
        actions,
    );

    assert!(matches!(
        normalized.get(normalized.len().saturating_sub(2)),
        Some(AgentAction::SynthesizeAnswer { evidence_refs })
            if evidence_refs == &vec!["step_1".to_string(), "step_2".to_string()]
    ));
    assert!(matches!(
        normalized.last(),
        Some(AgentAction::Respond { content }) if content == "{{last_output}}"
    ));
}

#[test]
fn scalar_path_route_treats_fs_search_query_as_name_pattern_when_action_missing() {
    let root = TempDirGuard::new("fs_search_name_contract");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    route.output_contract.delivery_required = false;
    let actions = vec![AgentAction::CallSkill {
        skill: "fs_search".to_string(),
        args: json!({
            "path": root_path,
            "query": "abcd",
        }),
    }];

    let normalized = enforce_output_contract_tool_args(Some(&route), "", None, actions);
    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "fs_search");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("find_name")
            );
            assert_eq!(
                args.get("pattern").and_then(|value| value.as_str()),
                Some("abcd")
            );
            assert_eq!(
                args.get("root").and_then(|value| value.as_str()),
                Some(root_path.as_str())
            );
        }
        other => panic!("expected fs_search action, got {other:?}"),
    }
}

#[test]
fn file_paths_route_preserves_grep_text_query_as_content_query() {
    let root = TempDirGuard::new("fs_search_grep_contract");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::FilePaths;
    route.output_contract.delivery_required = false;
    let actions = vec![AgentAction::CallSkill {
        skill: "fs_search".to_string(),
        args: json!({
            "action": "grep_text",
            "root": root_path,
            "query": "FirstLayerDecision",
            "max_results": 3
        }),
    }];

    let normalized = enforce_output_contract_tool_args(Some(&route), "", None, actions);
    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "fs_search");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("grep_text")
            );
            assert_eq!(
                args.get("query").and_then(Value::as_str),
                Some("FirstLayerDecision")
            );
            assert!(args.get("pattern").is_none());
            assert!(args.get("ext").is_none());
        }
        other => panic!("expected fs_search action, got {other:?}"),
    }
}

#[test]
fn system_basic_read_alias_is_normalized_to_read_range() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "read",
            "path": "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md",
        }),
    }];

    let normalized = normalize_system_basic_schema_aliases(actions);
    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "system_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("read_range")
            );
            assert_eq!(
                args.get("path").and_then(|value| value.as_str()),
                Some("scripts/nl_tests/fixtures/device_local/docs/release_checklist.md")
            );
        }
        other => panic!("expected system_basic read_range action, got {other:?}"),
    }
}

#[test]
fn system_basic_find_name_alias_is_normalized_to_find_path() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "find_name",
            "pattern": "missing.md",
            "max_results": 5,
        }),
    }];

    let normalized = normalize_system_basic_schema_aliases(actions);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "system_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("find_path")
            );
            assert_eq!(
                args.get("name").and_then(|value| value.as_str()),
                Some("missing.md")
            );
        }
        other => panic!("expected system_basic find_path action, got {other:?}"),
    }
}

#[test]
fn system_basic_check_exists_alias_is_normalized_to_path_batch_facts() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "check_exists",
            "path": "plan/extra_missing_repair_probe.md",
        }),
    }];

    let normalized = normalize_system_basic_schema_aliases(actions);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "system_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("path_batch_facts")
            );
            assert_eq!(
                args.get("paths").and_then(|value| value.as_array()),
                Some(&vec![json!("plan/extra_missing_repair_probe.md")])
            );
            assert!(args.get("path").is_none());
        }
        other => panic!("expected system_basic path_batch_facts action, got {other:?}"),
    }
}

#[test]
fn system_basic_check_exists_target_alias_keeps_batch_shape() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "check_exists",
            "target_path": "README.md",
        }),
    }];

    let normalized = normalize_system_basic_schema_aliases(actions);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "system_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("path_batch_facts")
            );
            assert_eq!(
                args.get("paths").and_then(|value| value.as_array()),
                Some(&vec![json!("README.md")])
            );
            assert!(args.get("target_path").is_none());
        }
        other => panic!("expected system_basic path_batch_facts action, got {other:?}"),
    }
}

#[test]
fn missing_read_range_path_uses_route_locator_hint() {
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "definitely_missing_system_basic_case.txt".to_string();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "find_path",
                "name": "definitely_missing_system_basic_case.txt",
            }),
        },
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "read_range",
                "mode": "head",
                "n": 3,
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
    ];

    let normalized = fill_missing_read_range_path_from_route_locator(Some(&route), actions);
    match &normalized[1] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "system_basic");
            assert_eq!(
                args.get("path").and_then(|value| value.as_str()),
                Some("definitely_missing_system_basic_case.txt")
            );
        }
        other => panic!("expected system_basic read_range action, got {other:?}"),
    }
}

#[test]
fn system_basic_read_range_lines_alias_becomes_range_bounds() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "read_range",
            "path": "README.md",
            "lines": "1-3",
        }),
    }];

    let normalized = normalize_system_basic_schema_aliases(actions);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "system_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("read_range")
            );
            assert_eq!(
                args.get("mode").and_then(|value| value.as_str()),
                Some("range")
            );
            assert_eq!(
                args.get("start_line").and_then(|value| value.as_u64()),
                Some(1)
            );
            assert_eq!(
                args.get("end_line").and_then(|value| value.as_u64()),
                Some(3)
            );
            assert_eq!(args.get("n").and_then(|value| value.as_u64()), Some(3));
            assert!(args.get("lines").is_none());
        }
        other => panic!("expected system_basic read_range action, got {other:?}"),
    }
}

#[test]
fn system_basic_read_range_range_tail_alias_becomes_mode_tail() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "read_range",
            "path": "logs/model_io.log",
            "range": "tail",
            "n": 4,
        }),
    }];

    let normalized = normalize_system_basic_schema_aliases(actions);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "system_basic");
            assert_eq!(
                args.get("mode").and_then(|value| value.as_str()),
                Some("tail")
            );
            assert_eq!(args.get("n").and_then(|value| value.as_u64()), Some(4));
            assert!(args.get("range").is_none());
        }
        other => panic!("expected system_basic read_range action, got {other:?}"),
    }
}

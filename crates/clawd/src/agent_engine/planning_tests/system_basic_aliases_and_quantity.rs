use super::*;

#[test]
fn system_basic_read_range_line_start_alias_becomes_range_bounds() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "read_range",
            "path": "README.md",
            "line_start": 1,
            "line_end": 8,
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
                Some(8)
            );
            assert!(args.get("line_start").is_none());
            assert!(args.get("line_end").is_none());
        }
        other => panic!("expected system_basic read_range action, got {other:?}"),
    }
}

#[test]
fn system_basic_read_alias_with_lines_becomes_range_bounds() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "read",
            "path": "README.md",
            "lines": [2, 4],
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
                Some(2)
            );
            assert_eq!(
                args.get("end_line").and_then(|value| value.as_u64()),
                Some(4)
            );
            assert_eq!(args.get("n").and_then(|value| value.as_u64()), Some(3));
            assert!(args.get("lines").is_none());
        }
        other => panic!("expected system_basic read_range action, got {other:?}"),
    }
}

#[test]
fn system_basic_read_range_negative_bounds_becomes_tail_count() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "read_range",
            "path": "logs/app.log",
            "start_line": -12,
            "end_line": -1,
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
            assert_eq!(args.get("n").and_then(|value| value.as_u64()), Some(12));
            assert!(args.get("start_line").is_none());
            assert!(args.get("end_line").is_none());
        }
        other => panic!("expected system_basic read_range action, got {other:?}"),
    }
}

#[test]
fn system_basic_read_range_negative_start_line_count_becomes_tail_count() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "read_range",
            "path": "logs/model_io.log",
            "start_line": -4,
            "line_count": 4,
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
            assert!(args.get("start_line").is_none());
            assert!(args.get("line_count").is_none());
        }
        other => panic!("expected system_basic read_range action, got {other:?}"),
    }
}

#[test]
fn system_basic_read_range_line_count_template_becomes_tail_count() {
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "file_lines_count",
                "path": "logs/model_io.log",
            }),
        },
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "read_range",
                "path": "logs/model_io.log",
                "start_line": "{{s1.result.line_count - 4}}",
                "end_line": "{{s1.result.line_count}}",
            }),
        },
    ];

    let normalized = strip_file_lines_count_before_tail_read_range(
        normalize_system_basic_schema_aliases(actions),
    );
    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "system_basic");
            assert_eq!(
                args.get("mode").and_then(|value| value.as_str()),
                Some("tail")
            );
            assert_eq!(args.get("n").and_then(|value| value.as_u64()), Some(5));
            assert!(args.get("start_line").is_none());
            assert!(args.get("end_line").is_none());
        }
        other => panic!("expected system_basic read_range action, got {other:?}"),
    }
}

#[test]
fn system_basic_list_dir_alias_is_normalized_to_inventory_dir() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "list_dir",
            "path": "scripts/nl_tests/fixtures/device_local/docs",
        }),
    }];

    let normalized = normalize_system_basic_schema_aliases(actions);
    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "system_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("inventory_dir")
            );
            assert_eq!(
                args.get("names_only").and_then(|value| value.as_bool()),
                Some(true)
            );
        }
        other => panic!("expected system_basic inventory_dir action, got {other:?}"),
    }
}

#[test]
fn system_basic_stat_paths_alias_is_normalized_to_path_batch_facts() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "stat_paths",
            "path": "configs/channels",
            "fields": ["path", "kind"],
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
                args.get("paths")
                    .and_then(Value::as_array)
                    .and_then(|paths| paths.first())
                    .and_then(Value::as_str),
                Some("configs/channels")
            );
            assert!(args.get("path").is_none());
        }
        other => panic!("expected system_basic path_batch_facts action, got {other:?}"),
    }
}

#[test]
fn system_basic_inventory_dir_dir_path_alias_becomes_path() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "inventory_dir",
            "dir_path": "scripts/nl_tests/fixtures/device_local/docs",
            "sort_by": "name",
        }),
    }];

    let normalized = normalize_system_basic_schema_aliases(actions);
    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "system_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("inventory_dir")
            );
            assert_eq!(
                args.get("path").and_then(|value| value.as_str()),
                Some("scripts/nl_tests/fixtures/device_local/docs")
            );
            assert!(args.get("dir_path").is_none());
        }
        other => panic!("expected system_basic inventory_dir action, got {other:?}"),
    }
}

#[test]
fn system_basic_count_dir_alias_is_normalized_to_count_inventory() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "count_dir",
            "directory_path": "document",
        }),
    }];

    let normalized = normalize_system_basic_schema_aliases(actions);
    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "system_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("count_inventory")
            );
            assert_eq!(
                args.get("path").and_then(|value| value.as_str()),
                Some("document")
            );
            assert!(args.get("directory_path").is_none());
        }
        other => panic!("expected system_basic count_inventory action, got {other:?}"),
    }
}

#[test]
fn system_basic_inventory_dir_extension_filter_implies_files_only() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "inventory_dir",
            "path": "document",
            "ext_filter": ".md",
            "names_only": true,
        }),
    }];

    let normalized = normalize_system_basic_schema_aliases(actions);
    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "system_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("inventory_dir")
            );
            assert_eq!(
                args.get("files_only").and_then(|value| value.as_bool()),
                Some(true)
            );
        }
        other => panic!("expected system_basic inventory_dir action, got {other:?}"),
    }
}

#[test]
fn system_basic_inventory_dir_normalizes_size_sort_aliases() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "inventory_dir",
            "path": "logs",
            "files_only": true,
            "sort_by": "size",
            "sort_order": "desc",
            "max_entries": 3,
        }),
    }];

    let normalized = normalize_system_basic_schema_aliases(actions);
    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "system_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("inventory_dir")
            );
            assert_eq!(
                args.get("sort_by").and_then(|value| value.as_str()),
                Some("size_desc")
            );
        }
        other => panic!("expected system_basic inventory_dir action, got {other:?}"),
    }
}

#[test]
fn hidden_entries_contract_forces_inventory_dir_include_hidden() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "inventory_dir",
            "path": ".",
            "names_only": true,
        }),
    }];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::HiddenEntriesCheck;

    let normalized = normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(1),
        "检查当前目录有没有隐藏文件，只回答有或没有，并补 3 个例子",
        None,
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "list_dir");
    assert_eq!(
        args.get("include_hidden").and_then(Value::as_bool),
        Some(true)
    );
}

#[test]
fn structured_scalar_compare_plan_appends_synthesize_answer() {
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "extract_fields",
                "path": "UI/package.json",
                "field_paths": ["name"]
            }),
        },
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "extract_field",
                "path": "crates/clawd/Cargo.toml",
                "field_path": "package.name"
            }),
        },
    ];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.resolved_intent =
            "UI/package.json 里的 name 和 crates/clawd/Cargo.toml 里的 package.name 一样吗？只回答一样或不一样"
                .to_string();

    let normalized = super::super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        &route.resolved_intent,
        None,
        actions,
    );
    assert!(matches!(
        normalized.iter().find(|action| matches!(action, AgentAction::SynthesizeAnswer { .. })),
        Some(AgentAction::SynthesizeAnswer { evidence_refs })
            if evidence_refs == &vec!["step_1".to_string(), "step_2".to_string()]
    ));
    assert!(matches!(
        normalized.last(),
        Some(AgentAction::Respond { content }) if content == "{{last_output}}"
    ));
}

#[test]
fn free_quantity_compare_plan_appends_synthesize_for_compare_paths() {
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "compare_paths",
            "left_path": "README.md",
            "right_path": "README.zh-CN.md",
        }),
    }];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.route_reason = "quantity_comparison_requires_model_language_synthesis".to_string();
    route.resolved_intent =
        "Compare README.md and README.zh-CN.md by size and include a bounded synthesis."
            .to_string();

    let normalized = super::super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        &route.resolved_intent,
        None,
        actions,
    );

    assert!(matches!(
        normalized.iter().find(|action| matches!(action, AgentAction::SynthesizeAnswer { .. })),
        Some(AgentAction::SynthesizeAnswer { evidence_refs })
            if evidence_refs == &vec!["step_1".to_string()]
    ));
    assert!(matches!(
        normalized.last(),
        Some(AgentAction::Respond { content }) if content == "{{last_output}}"
    ));
}

#[test]
fn quantity_comparison_single_directory_count_observation_is_nonrecursive() {
    let root = TempDirGuard::new("quantity_single_dir_count_nonrecursive");
    fs::create_dir_all(root.path.join("nested")).expect("create nested");
    fs::write(root.path.join("top.txt"), "top").expect("write top");
    fs::write(root.path.join("nested/deep.txt"), "deep").expect("write deep");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_path.clone();

    let actions = file_facts_auto_locator_observation_plan(Some(&route), Some(root_path.as_str()))
        .expect("directory quantity observation plan");

    let count_action = actions
        .iter()
        .find(|action| planned_call_is(action, "fs_basic", "count_entries"))
        .expect("count_entries action");
    let args = expect_planned_call(count_action, "fs_basic", "count_entries");
    assert_eq!(args.get("recursive").and_then(Value::as_bool), Some(false));
    assert_eq!(args.get("count_files").and_then(Value::as_bool), Some(true));
    assert_eq!(args.get("count_dirs").and_then(Value::as_bool), Some(true));
}

#[test]
fn structured_scalar_compare_repairs_whole_file_read_plan() {
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: serde_json::json!({ "path": "UI/package.json" }),
        },
        AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: serde_json::json!({ "path": "crates/clawd/Cargo.toml" }),
        },
    ];
    let normalized = super::super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "读取两个字段并比较",
        None,
        actions,
    );

    assert!(should_force_actionable_plan_repair(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        &normalized
    ));
    assert_eq!(
        plan_repair_reason(
            &test_state(),
            Some(&route),
            &LoopState::new(2),
            Some(&normalized)
        ),
        "structured_scalar_compare_requires_extract_fields"
    );
}

#[test]
fn structured_scalar_compare_repair_can_add_text_after_prior_scalar_extract() {
    use crate::executor::{StepExecutionResult, StepExecutionStatus};

    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    let mut loop_state = LoopState::new(2);
    loop_state.round_no = 2;
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            serde_json::json!({
                "action": "extract_field",
                "path": "Cargo.toml",
                "field_path": "workspace.package.version",
                "value": "0.1.7",
                "value_text": "0.1.7"
            })
            .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let actions = vec![AgentAction::CallSkill {
        skill: "fs_search".to_string(),
        args: serde_json::json!({
            "action": "grep_text",
            "root": "README.md",
            "query": "0.1.7",
            "max_results": 5
        }),
    }];

    assert!(
        !should_force_actionable_plan_repair(&test_state(), Some(&route), &loop_state, &actions),
        "unexpected repair reason: {}",
        plan_repair_reason(&test_state(), Some(&route), &loop_state, Some(&actions))
    );
}

#[test]
fn recent_scalar_equality_repair_counts_prior_config_basic_field_extract() {
    use crate::executor::{StepExecutionResult, StepExecutionStatus};

    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RecentScalarEqualityCheck;
    let mut loop_state = LoopState::new(4);
    loop_state.round_no = 2;
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "config_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            serde_json::json!({
                "extra": {
                    "action": "read_field",
                    "path": "Cargo.toml",
                    "field_path": "workspace.package.version",
                    "field_value": "0.1.7"
                },
                "text": "0.1.7"
            })
            .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "grep_text",
            "root": "README.md",
            "query": "0.1.7",
            "max_results": 5
        }),
    }];

    assert!(
        !should_force_actionable_plan_repair(&test_state(), Some(&route), &loop_state, &actions),
        "unexpected repair reason: {}",
        plan_repair_reason(&test_state(), Some(&route), &loop_state, Some(&actions))
    );
}

#[test]
fn recent_scalar_equality_repair_compacts_prior_structured_text_read() {
    use crate::executor::{StepExecutionResult, StepExecutionStatus};

    let root = TempDirGuard::new("recent_scalar_prior_structured_text_read");
    fs::create_dir_all(root.path.join("scripts/nl_tests/fixtures/device_local"))
        .expect("fixture dir");
    fs::create_dir_all(root.path.join("crates/clawd")).expect("clawd dir");
    let package_path = root
        .path
        .join("scripts/nl_tests/fixtures/device_local/package.json");
    let cargo_path = root.path.join("crates/clawd/Cargo.toml");
    fs::write(
        &package_path,
        r#"{"name":"rustclaw-nl-fixture","version":"0.1.0"}"#,
    )
    .expect("write package json");
    fs::write(
        &cargo_path,
        r#"[package]
name = "clawd"
version = "0.1.0"
"#,
    )
    .expect("write cargo manifest");

    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let package_path_text = package_path.display().to_string();
    let cargo_path_text = cargo_path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RecentScalarEqualityCheck;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = format!("{package_path_text}|{cargo_path_text}");
    route.resolved_intent =
        "Read name field from scripts/nl_tests/fixtures/device_local/package.json and package.name from crates/clawd/Cargo.toml, compare the two values, and output one line with both names followed by same or different"
            .to_string();

    let mut loop_state = LoopState::new(4);
    loop_state.round_no = 2;
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            serde_json::json!({
                "extra": {
                    "action": "read_text_range",
                    "path": package_path_text.clone(),
                    "resolved_path": package_path_text.clone(),
                    "excerpt": "{\"name\":\"rustclaw-nl-fixture\"}"
                },
                "text": "{\"name\":\"rustclaw-nl-fixture\"}"
            })
            .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let actions = vec![
        AgentAction::CallTool {
            tool: "config_basic".to_string(),
            args: serde_json::json!({
                "action": "read_field",
                "path": cargo_path_text.clone(),
                "field_path": "package.name"
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["step_1".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    assert_eq!(
        super::super::executed_structured_text_read_paths(&loop_state),
        vec![package_path_text.clone()]
    );
    let prior_action = super::super::structured_scalar_read_action_for_target(
        &state,
        &route,
        route.resolved_intent.as_str(),
        package_path_text.as_str(),
    )
    .expect("package.json name selector");
    let prior_args = expect_planned_call(&prior_action, "config_basic", "read_field");
    assert_eq!(
        prior_args.get("field_path").and_then(Value::as_str),
        Some("name")
    );

    let compacted = super::super::add_prior_structured_text_field_read_for_scalar_compare(
        &state,
        Some(&route),
        &loop_state,
        route.resolved_intent.as_str(),
        None,
        actions.clone(),
    );
    let compacted_first = expect_planned_call(&compacted[0], "config_basic", "read_field");
    assert_eq!(
        compacted_first.get("path").and_then(Value::as_str),
        Some(package_path_text.as_str())
    );

    let normalized = super::super::normalize_planned_actions(
        &state,
        Some(&route),
        &loop_state,
        route.resolved_intent.as_str(),
        None,
        actions,
    );

    let first_args = expect_planned_call(&normalized[0], "config_basic", "read_field");
    assert_eq!(
        first_args.get("path").and_then(Value::as_str),
        Some(package_path_text.as_str())
    );
    assert_eq!(
        first_args.get("field_path").and_then(Value::as_str),
        Some("name")
    );
    let second_args = expect_planned_call(&normalized[1], "config_basic", "read_field");
    assert_eq!(
        second_args.get("path").and_then(Value::as_str),
        Some(cargo_path_text.as_str())
    );
    assert_eq!(
        second_args.get("field_path").and_then(Value::as_str),
        Some("package.name")
    );
    assert_eq!(normalized.len(), 2);
    assert!(
        !should_force_actionable_plan_repair(&state, Some(&route), &loop_state, &normalized),
        "unexpected repair reason: {}",
        plan_repair_reason(&state, Some(&route), &loop_state, Some(&normalized))
    );
}

#[test]
fn structured_scalar_compare_allows_text_read_after_wrapped_inventory_evidence() {
    use crate::executor::{StepExecutionResult, StepExecutionStatus};

    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.output_contract.requires_content_evidence = true;
    let mut loop_state = LoopState::new(4);
    loop_state.round_no = 2;
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            serde_json::json!({
                "extra": {
                    "action": "inventory_dir",
                    "path": "prompts/schemas",
                    "resolved_path": "/repo/prompts/schemas",
                    "counts": {"files": 22, "dirs": 0, "total": 22},
                    "entries": [
                        {
                            "kind": "file",
                            "name": "intent_normalizer.schema.json",
                            "path": "prompts/schemas/intent_normalizer.schema.json",
                            "size_bytes": 14775
                        }
                    ],
                    "size_summary": {
                        "largest_file": {
                            "kind": "file",
                            "name": "intent_normalizer.schema.json",
                            "path": "prompts/schemas/intent_normalizer.schema.json",
                            "size_bytes": 14775
                        }
                    }
                },
                "text": "{\"action\":\"inventory_dir\",\"path\":\"prompts/schemas\"}"
            })
            .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": "read_text_range",
                "mode": "head",
                "path": "prompts/schemas/intent_normalizer.schema.json",
                "n": 50
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = super::super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &loop_state,
        "summarize the largest schema after listing schema files",
        None,
        actions,
    );

    assert!(
        !should_force_actionable_plan_repair(&test_state(), Some(&route), &loop_state, &normalized),
        "unexpected repair reason: {}",
        plan_repair_reason(&test_state(), Some(&route), &loop_state, Some(&normalized))
    );
}

#[test]
fn structured_scalar_compare_keeps_two_structured_extracts_for_strict_shape() {
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "extract_fields",
                "path": "UI/package.json",
                "field_paths": ["name"]
            }),
        },
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "extract_fields",
                "path": "crates/clawd/Cargo.toml",
                "field_paths": ["package.name"]
            }),
        },
    ];
    let normalized = super::super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "读取两个字段并比较",
        None,
        actions,
    );

    assert!(matches!(
        normalized.iter().find(|action| matches!(action, AgentAction::SynthesizeAnswer { .. })),
        Some(AgentAction::SynthesizeAnswer { evidence_refs })
            if evidence_refs
                == &vec!["step_1".to_string(), "step_2".to_string()]
    ));
    assert!(matches!(
        normalized.last(),
        Some(AgentAction::Respond { content }) if content == "{{last_output}}"
    ));
    assert!(!should_force_actionable_plan_repair(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        &normalized
    ));
}

#[test]
fn structured_scalar_compare_accepts_two_directory_inventory_observations() {
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "list_dir",
                "path": "scripts/nl_tests/fixtures/device_local/docs"
            }),
        },
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "list_dir",
                "path": "scripts/nl_tests/fixtures/device_local/logs"
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string(), "s1".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = super::super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "先数 docs 直接子项数量，再数 logs 直接子项数量，最后一句中文说哪个更多",
        None,
        actions,
    );

    assert!(planned_call_is(&normalized[0], "fs_basic", "list_dir"));
    assert!(planned_call_is(&normalized[1], "fs_basic", "list_dir"));
    assert!(!should_force_actionable_plan_repair(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        &normalized
    ));
}

#[test]
fn structured_scalar_compare_accepts_path_batch_facts_for_file_metadata() {
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: serde_json::json!({
            "action": "path_batch_facts",
            "paths": ["Cargo.lock", "Cargo.toml"]
        }),
    }];
    let normalized = super::super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "比较 Cargo.lock 和 Cargo.toml 的大小",
        None,
        actions,
    );

    assert!(matches!(
        normalized.iter().find(|action| matches!(action, AgentAction::SynthesizeAnswer { .. })),
        Some(AgentAction::SynthesizeAnswer { evidence_refs })
            if evidence_refs == &vec!["step_1".to_string()]
    ));
    assert!(matches!(
        normalized.last(),
        Some(AgentAction::Respond { content }) if content == "{{last_output}}"
    ));
    assert!(!should_force_actionable_plan_repair(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        &normalized
    ));
}

#[test]
fn structured_scalar_compare_one_sentence_accepts_path_batch_facts_metadata_evidence() {
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "path_batch_facts",
                "paths": ["README.md", "AGENTS.md"],
                "fields": ["size_bytes"]
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["step_1".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    assert!(!should_force_actionable_plan_repair(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        &actions
    ));
    assert_ne!(
        plan_repair_reason(
            &test_state(),
            Some(&route),
            &LoopState::new(2),
            Some(&actions)
        ),
        "content_evidence_requires_content_observation"
    );
}

#[test]
fn structured_scalar_compare_free_shape_accepts_path_batch_facts_metadata_evidence() {
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "Cargo.toml | Cargo.lock".to_string();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "path_batch_facts",
                "paths": ["Cargo.toml", "Cargo.lock"],
                "fields": ["size_bytes"]
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["step_1".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    assert!(!should_force_actionable_plan_repair(
        &test_state(),
        Some(&route),
        &LoopState::new(1),
        &actions
    ));
    assert_ne!(
        plan_repair_reason(
            &test_state(),
            Some(&route),
            &LoopState::new(1),
            Some(&actions)
        ),
        "content_evidence_requires_content_observation"
    );
}

#[test]
fn quantity_compare_rewrites_directory_name_searches_to_dir_compare() {
    let root = TempDirGuard::new("quantity_dir_compare");
    fs::create_dir_all(root.path.join("tmp/bundle_src/nested")).expect("create left");
    fs::create_dir_all(root.path.join("tmp/dynamic_guard_unpack_case/nested"))
        .expect("create right");
    fs::write(root.path.join("tmp/bundle_src/notes.txt"), "same\n").expect("write left");
    fs::write(
        root.path.join("tmp/dynamic_guard_unpack_case/notes.txt"),
        "same\n",
    )
    .expect("write right");

    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    state.skill_rt.locator_scan_max_files = 5000;

    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = root.path.display().to_string();

    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": "find_entries",
                "root": ".",
                "pattern": "bundle_src"
            }),
        },
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": "find_entries",
                "root": ".",
                "pattern": "dynamic_guard_unpack_case"
            }),
        },
    ];

    let normalized = normalize_planned_actions_with_original_and_context(
        &state,
        Some(&route),
        &LoopState::new(2),
        "compare bundle_src and dynamic_guard_unpack_case recursively",
        None,
        None,
        None,
        actions,
    );

    assert_eq!(normalized.len(), 1, "normalized: {normalized:?}");
    let args = expect_planned_call(&normalized[0], "system_basic", "dir_compare");
    assert_eq!(
        args.get("left_path").and_then(Value::as_str),
        Some("tmp/bundle_src")
    );
    assert_eq!(
        args.get("right_path").and_then(Value::as_str),
        Some("tmp/dynamic_guard_unpack_case")
    );
    assert_eq!(args.get("recursive").and_then(Value::as_bool), Some(true));
}

#[test]
fn quantity_compare_directory_pair_uses_deterministic_dir_compare_plan() {
    let root = TempDirGuard::new("quantity_dir_compare_locator");
    let left = root.path.join("tmp/bundle_src");
    let right = root.path.join("tmp/dynamic_guard_unpack_case");
    fs::create_dir_all(&left).expect("left");
    fs::create_dir_all(&right).expect("right");

    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::direct_answer(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = format!("{} | {}", left.display(), right.display());

    let plan = directory_compare_locator_deterministic_plan_result(
        &state,
        "compare two directories recursively",
        Some(&route),
        &LoopState::new(1),
    )
    .expect("deterministic dir compare plan");

    assert_eq!(plan.steps.len(), 1);
    let action = plan.steps[0].to_agent_action().expect("action");
    let args = expect_planned_call(&action, "system_basic", "dir_compare");
    let expected_left = left.canonicalize().unwrap().display().to_string();
    let expected_right = right.canonicalize().unwrap().display().to_string();
    assert_eq!(
        args.get("left_path").and_then(Value::as_str),
        Some(expected_left.as_str())
    );
    assert_eq!(
        args.get("right_path").and_then(Value::as_str),
        Some(expected_right.as_str())
    );
}

#[test]
fn directory_pair_locator_uses_dir_compare_even_without_quantity_semantic() {
    let root = TempDirGuard::new("directory_pair_compare_locator_no_semantic");
    let left = root.path.join("tmp/bundle_src");
    let right = root.path.join("tmp/dynamic_guard_unpack_case");
    fs::create_dir_all(&left).expect("left");
    fs::create_dir_all(&right).expect("right");

    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = format!("{} | {}", left.display(), right.display());

    let plan = directory_compare_locator_deterministic_plan_result(
        &state,
        "compare two directory targets",
        Some(&route),
        &LoopState::new(1),
    )
    .expect("deterministic dir compare plan");

    assert_eq!(plan.steps.len(), 1);
    let action = plan.steps[0].to_agent_action().expect("action");
    let args = expect_planned_call(&action, "system_basic", "dir_compare");
    assert_eq!(
        args.get("left_path").and_then(Value::as_str),
        Some(left.canonicalize().unwrap().to_string_lossy().as_ref())
    );
    assert_eq!(
        args.get("right_path").and_then(Value::as_str),
        Some(right.canonicalize().unwrap().to_string_lossy().as_ref())
    );
}

#[test]
fn dir_compare_plan_rewrites_unique_directory_basenames_to_paths() {
    let root = TempDirGuard::new("dir_compare_unique_basename_rewrite");
    let left = root.path.join("fixtures/tmp/bundle_src");
    let right = root.path.join("fixtures/tmp/dynamic_guard_unpack_case");
    fs::create_dir_all(&left).expect("left");
    fs::create_dir_all(&right).expect("right");

    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let actions = vec![AgentAction::CallTool {
        tool: "system_basic".to_string(),
        args: serde_json::json!({
            "action": "dir_compare",
            "left_path": "bundle_src",
            "right_path": "dynamic_guard_unpack_case",
            "recursive": true,
            "max_diffs": 20,
        }),
    }];

    let normalized = super::super::normalize_planned_actions(
        &state,
        None,
        &LoopState::new(1),
        "",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 1);
    let args = expect_planned_call(&normalized[0], "system_basic", "dir_compare");
    let expected_left = left.canonicalize().unwrap().display().to_string();
    let expected_right = right.canonicalize().unwrap().display().to_string();
    assert_eq!(
        args.get("left_path").and_then(Value::as_str),
        Some(expected_left.as_str())
    );
    assert_eq!(
        args.get("right_path").and_then(Value::as_str),
        Some(expected_right.as_str())
    );
}

#[test]
fn compare_paths_plan_rewrites_to_system_dir_compare_with_resolved_dirs() {
    let root = TempDirGuard::new("compare_paths_to_dir_compare_rewrite");
    for idx in 0..2500 {
        fs::create_dir_all(root.path.join(format!("aaa_filler_{idx:04}"))).expect("filler");
    }
    let left = root.path.join("fixtures/tmp/bundle_src");
    let right = root.path.join("fixtures/tmp/dynamic_guard_unpack_case");
    fs::create_dir_all(&left).expect("left");
    fs::create_dir_all(&right).expect("right");

    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    state.skill_rt.locator_scan_max_files = 10;
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "compare_paths",
            "paths": ["bundle_src", "dynamic_guard_unpack_case"],
            "recursive": true
        }),
    }];

    let normalized = super::super::normalize_planned_actions(
        &state,
        None,
        &LoopState::new(1),
        "",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 1);
    let args = expect_planned_call(&normalized[0], "system_basic", "dir_compare");
    let expected_left = left.canonicalize().unwrap().display().to_string();
    let expected_right = right.canonicalize().unwrap().display().to_string();
    assert_eq!(
        args.get("left_path").and_then(Value::as_str),
        Some(expected_left.as_str())
    );
    assert_eq!(
        args.get("right_path").and_then(Value::as_str),
        Some(expected_right.as_str())
    );
    assert!(args.get("paths").is_none());
}

use super::*;

#[test]
fn normalize_planned_actions_resolves_call_capability_before_policy_gate() {
    let state = test_state_with_registry();
    let actions = vec![AgentAction::CallCapability {
        capability: "filesystem.list_entries".to_string(),
        args: json!({
            "path": ".",
            "names_only": true,
        }),
    }];

    let normalized = normalize_planned_actions(&state, None, &LoopState::new(1), "", None, actions);

    assert_eq!(normalized.len(), 1);
    let args = expect_planned_call(&normalized[0], "fs_basic", "list_dir");
    assert_eq!(args.get("path").and_then(Value::as_str), Some("."));
    assert_eq!(args.get("names_only").and_then(Value::as_bool), Some(true));
}

#[test]
fn normalize_planned_actions_resolves_action_ref_call_capability_before_policy_gate() {
    let state = test_state_with_registry();
    let actions = vec![AgentAction::CallCapability {
        capability: "fs_basic.read_text_range".to_string(),
        args: json!({
            "path": "scripts/nl_tests/fixtures/device_local/logs/app.log",
            "mode": "tail",
            "n": 20,
        }),
    }];

    let normalized = normalize_planned_actions(&state, None, &LoopState::new(1), "", None, actions);

    assert_eq!(normalized.len(), 1);
    let args = expect_planned_call(&normalized[0], "fs_basic", "read_text_range");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some("scripts/nl_tests/fixtures/device_local/logs/app.log")
    );
    assert_eq!(args.get("mode").and_then(Value::as_str), Some("tail"));
    assert_eq!(args.get("n").and_then(Value::as_i64), Some(20));
}

#[test]
fn normalize_planned_actions_applies_skill_arg_aliases_before_verifier() {
    let state = test_state_with_registry();
    let actions = vec![AgentAction::CallSkill {
        skill: "image_edit".to_string(),
        args: json!({
            "image": "https://example.test/rust.png",
            "prompt": "pixel art style",
            "output_path": "document/rust_icon_pixel_smoke.png"
        }),
    }];

    let normalized = normalize_planned_actions(&state, None, &LoopState::new(1), "", None, actions);

    assert_eq!(normalized.len(), 1);
    let AgentAction::CallSkill { skill, args } = &normalized[0] else {
        panic!("expected image_edit call, got {:?}", normalized[0]);
    };
    assert_eq!(skill, "image_edit");
    assert_eq!(
        args.get("instruction").and_then(Value::as_str),
        Some("pixel art style")
    );
}

#[test]
fn compound_capability_plan_preserves_stat_paths_supporting_content_read() {
    let state = test_state_with_registry();
    let mut route = base_route_result();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::WorkspaceProjectSummary;
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/home/guagua/rustclaw/README.md".to_string();
    let actions = vec![
        AgentAction::CallCapability {
            capability: "filesystem.stat_paths".to_string(),
            args: json!({
                "paths": ["/home/guagua/rustclaw/not_real_20260511"],
            }),
        },
        AgentAction::CallCapability {
            capability: "filesystem.read_text_range".to_string(),
            args: json!({
                "path": "/home/guagua/rustclaw/README.md",
                "mode": "head",
                "n": 80,
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["s1".to_string(), "s2".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized =
        normalize_planned_actions(&state, Some(&route), &LoopState::new(1), "", None, actions);

    let stat_action = normalized
        .iter()
        .find(|action| planned_call_is(action, "fs_basic", "stat_paths"))
        .unwrap_or_else(|| {
            panic!("expected fs_basic.stat_paths to be preserved, got {normalized:#?}")
        });
    let stat_args = expect_planned_call(stat_action, "fs_basic", "stat_paths");
    assert_eq!(
        stat_args
            .get("paths")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(1)
    );
    let read_action = normalized
        .iter()
        .find(|action| planned_call_is(action, "fs_basic", "read_text_range"))
        .expect("expected fs_basic.read_text_range to be preserved");
    let read_args = expect_planned_call(read_action, "fs_basic", "read_text_range");
    assert_eq!(
        read_args.get("path").and_then(Value::as_str),
        Some("/home/guagua/rustclaw/README.md")
    );
}

#[test]
fn command_output_summary_preserves_fs_stat_paths_observation() {
    let state = test_state_with_registry();
    let mut route = base_route_result();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::CommandOutputSummary;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/home/guagua/rustclaw/configs/config.toml".to_string();

    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "stat_paths",
                "paths": ["README.md"],
            }),
        },
        AgentAction::CallTool {
            tool: "system_basic".to_string(),
            args: json!({
                "action": "runtime_status",
                "kind": "current_working_directory",
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized =
        normalize_planned_actions(&state, Some(&route), &LoopState::new(2), "", None, actions);

    let first = normalized
        .first()
        .unwrap_or_else(|| panic!("expected actions, got {normalized:#?}"));
    let stat_args = expect_planned_call(first, "fs_basic", "stat_paths");
    assert_eq!(
        stat_args
            .get("paths")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(1)
    );
    assert!(
        !normalized
            .iter()
            .any(|action| planned_call_is(action, "git_basic", "status")),
        "fs path observation must not be replaced by git status: {normalized:#?}"
    );
}

#[test]
fn normalize_planned_actions_keeps_unresolved_call_capability_for_verifier() {
    let state = test_state_with_registry();
    let actions = vec![AgentAction::CallCapability {
        capability: "unknown.example".to_string(),
        args: json!({}),
    }];

    let normalized = normalize_planned_actions(&state, None, &LoopState::new(1), "", None, actions);

    assert_eq!(normalized.len(), 1);
    assert!(matches!(
        &normalized[0],
        AgentAction::CallCapability { capability, .. } if capability == "unknown.example"
    ));
}

#[test]
fn structured_text_read_range_without_bounds_reads_broader_context() {
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "read_text_range",
            "path": "prompts/schemas/direct_answer_gate.schema.json",
            "format": "text",
        }),
    }];

    let normalized = broaden_default_read_range_for_structured_text(actions);

    let args = expect_planned_call(&normalized[0], "fs_basic", "read_text_range");
    assert_eq!(args.get("mode").and_then(Value::as_str), Some("head"));
    assert_eq!(args.get("n").and_then(Value::as_u64), Some(500));
}

#[test]
fn structured_text_read_range_keeps_explicit_bounds() {
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "read_text_range",
            "path": "config.json",
            "start_line": 1,
            "end_line": 3,
        }),
    }];

    let normalized = broaden_default_read_range_for_structured_text(actions);

    let args = expect_planned_call(&normalized[0], "fs_basic", "read_text_range");
    assert!(args.get("mode").is_none());
    assert!(args.get("n").is_none());
}

#[test]
fn structured_text_full_mode_without_n_reads_broader_context() {
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "read_text_range",
            "path": "prompts/schemas/direct_answer_gate.schema.json",
            "mode": "full",
        }),
    }];

    let normalized = broaden_default_read_range_for_structured_text(actions);

    let args = expect_planned_call(&normalized[0], "fs_basic", "read_text_range");
    assert_eq!(args.get("mode").and_then(Value::as_str), Some("full"));
    assert_eq!(args.get("n").and_then(Value::as_u64), Some(500));
}

#[test]
fn structured_text_tail_mode_keeps_default_window() {
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "read_text_range",
            "path": "config.json",
            "mode": "tail",
        }),
    }];

    let normalized = broaden_default_read_range_for_structured_text(actions);

    let args = expect_planned_call(&normalized[0], "fs_basic", "read_text_range");
    assert_eq!(args.get("mode").and_then(Value::as_str), Some("tail"));
    assert!(args.get("n").is_none());
}

#[test]
fn plain_text_read_range_keeps_default_bounds() {
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "read_text_range",
            "path": "README.md",
        }),
    }];

    let normalized = broaden_default_read_range_for_structured_text(actions);

    let args = expect_planned_call(&normalized[0], "fs_basic", "read_text_range");
    assert!(args.get("mode").is_none());
    assert!(args.get("n").is_none());
}

#[test]
fn contract_scoped_planner_skill_scope_uses_allowed_action_skills() {
    let mut route = base_route_result();
    route.output_contract.semantic_kind = OutputSemanticKind::FilePaths;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;

    let scope = contract_scoped_planner_skill_scope(Some(&route)).expect("contract scope");

    assert_eq!(scope.len(), 1);
    assert!(scope.contains("fs_basic"));
}

#[test]
fn lightweight_contract_scope_caps_broad_command_summary_to_preferred_skills() {
    let mut route = base_route_result();
    route.output_contract.semantic_kind = OutputSemanticKind::CommandOutputSummary;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;

    assert!(contract_scoped_planner_skill_scope(Some(&route)).is_none());

    let scope =
        contract_scoped_lightweight_planner_skill_scope(Some(&route)).expect("lightweight scope");
    assert!(scope.len() <= 8);
    assert!(scope.contains("run_cmd"));
    assert!(scope.contains("process_basic"));
    assert!(scope.contains("system_basic"));
    assert!(!scope.contains("kb"));
    assert!(!scope.contains("archive_basic"));
}

#[test]
fn contract_scoped_planner_skill_scope_leaves_unclassified_routes_open() {
    let route = base_route_result();

    assert!(contract_scoped_planner_skill_scope(Some(&route)).is_none());
}

#[test]
fn sqlite_table_listing_route_rewrites_text_read_plan_to_db_basic_list_tables() {
    let mut route = base_route_result();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::SqliteTableListing;
    route.output_contract.locator_hint = "/tmp/app.sqlite".to_string();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "read_range",
                "path": "/tmp/app.sqlite",
                "command": "sqlite3 /tmp/app.sqlite \"SELECT name FROM sqlite_master WHERE type='table';\""
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let rewritten = rewrite_sqlite_table_listing_plan_to_db_basic(
        Some(&route),
        Some("/tmp/app.sqlite"),
        false,
        actions,
    );

    match &rewritten[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "db_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("list_tables")
            );
            assert_eq!(
                args.get("db_path").and_then(|value| value.as_str()),
                Some("/tmp/app.sqlite")
            );
            assert!(args.get("sql").is_none());
        }
        other => panic!("expected db_basic action, got {other:?}"),
    }
    assert!(matches!(rewritten[1], AgentAction::SynthesizeAnswer { .. }));
    assert!(matches!(rewritten[2], AgentAction::Respond { .. }));
}

#[test]
fn sqlite_binary_text_read_fallback_rewrites_to_db_basic_list_tables_without_semantic_kind() {
    let mut route = base_route_result();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_hint = "/tmp/app.sqlite".to_string();
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": "/tmp/app.sqlite",
                "mode": "head",
                "n": 120
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
    ];

    let rewritten = rewrite_sqlite_table_listing_plan_to_db_basic(
        Some(&route),
        Some("/tmp/app.sqlite"),
        false,
        actions,
    );

    match &rewritten[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "db_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("list_tables")
            );
            assert_eq!(
                args.get("db_path").and_then(Value::as_str),
                Some("/tmp/app.sqlite")
            );
        }
        other => panic!("expected db_basic action, got {other:?}"),
    }
    assert!(matches!(rewritten[1], AgentAction::SynthesizeAnswer { .. }));
}

#[test]
fn existence_path_summary_plan_inserts_bounded_content_observation() {
    let state = test_state();
    let loop_state = LoopState::new(1);
    let mut route = base_route_result();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPathSummary;
    route.output_contract.locator_hint = "rustclaw.service".to_string();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "path_batch_facts",
                "paths": ["/tmp/rustclaw.service"],
                "include_missing": true
            }),
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &loop_state,
        "check service file and summarize its purpose",
        Some("/tmp/rustclaw.service"),
        actions,
    );

    assert!(normalized.iter().any(|action| {
        action_capability_and_action(action, "fs_basic", "read_text_range").is_some_and(|args| {
            args.get("path").and_then(Value::as_str) == Some("/tmp/rustclaw.service")
        })
    }));
    assert!(normalized.iter().any(|action| {
        matches!(
            action,
            AgentAction::SynthesizeAnswer { evidence_refs }
                if evidence_refs == &vec!["step_1".to_string(), "step_2".to_string()]
        )
    }));
    assert!(matches!(
        normalized.last(),
        Some(AgentAction::Respond { content }) if content == "{{last_output}}"
    ));
}

#[test]
fn existence_path_summary_workspace_search_scope_does_not_read_directory() {
    let mut state = test_state();
    let root = TempDirGuard::new("existence_path_summary_workspace_scope");
    state.skill_rt.workspace_root = root.path.clone();
    let loop_state = LoopState::new(1);
    let mut route = base_route_result();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPathSummary;
    route.output_contract.locator_hint = "rustclaw.service".to_string();
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "find_entries",
                "root": root.path.display().to_string(),
                "pattern": "rustclaw.service",
                "kind": "file",
                "recursive": true,
                "max_results": 20
            }),
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &loop_state,
        "",
        Some(root.path.to_str().expect("utf8 temp path")),
        actions,
    );

    assert!(!normalized.iter().any(|action| planned_call_is(
        action,
        "fs_basic",
        "read_text_range"
    )));
    assert!(normalized.iter().any(|action| {
        matches!(
            action,
            AgentAction::SynthesizeAnswer { evidence_refs }
                if evidence_refs == &vec!["last_output".to_string()]
        )
    }));
    assert!(matches!(
        normalized.last(),
        Some(AgentAction::Respond { content }) if content == "{{last_output}}"
    ));
}

#[test]
fn existence_path_summary_metadata_placeholder_does_not_force_file_read() {
    let state = test_state();
    let loop_state = LoopState::new(1);
    let mut route = base_route_result();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPathSummary;
    route.output_contract.locator_hint = "data/rustclaw.db".to_string();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "path_batch_facts",
                "paths": ["/tmp/rustclaw.db"],
                "include_missing": true
            }),
        },
        AgentAction::Respond {
            content: "文件存在，大小为 {{size}} 字节。".to_string(),
        },
    ];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &loop_state,
        "check whether the file exists and report its size",
        Some("/tmp/rustclaw.db"),
        actions,
    );

    assert!(!normalized.iter().any(|action| {
        matches!(
            action,
            AgentAction::CallSkill { skill, args }
                if skill == "system_basic"
                    && args.get("action").and_then(Value::as_str) == Some("read_range")
        )
    }));
    assert!(normalized.iter().any(|action| {
        matches!(
            action,
            AgentAction::SynthesizeAnswer { evidence_refs }
                if evidence_refs == &vec!["step_1".to_string()]
                    || evidence_refs == &vec!["last_output".to_string()]
        )
    }));
    assert!(matches!(
        normalized.last(),
        Some(AgentAction::Respond { content }) if content == "{{last_output}}"
    ));
}

#[test]
fn existence_with_path_metadata_batch_answer_does_not_force_content_repair() {
    let state = test_state();
    let loop_state = LoopState::new(1);
    let mut route = base_route_result();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_hint = "README.md".to_string();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "path_batch_facts",
                "paths": ["/home/guagua/rustclaw/README.md"],
                "include_missing": true,
                "fields": ["exists", "size"]
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    assert!(!should_force_actionable_plan_repair(
        &state,
        Some(&route),
        &loop_state,
        &actions
    ));
    assert!(can_fallback_to_initial_plan_after_repair_failure(
        &state,
        Some(&route),
        &loop_state,
        &actions
    ));
}

#[test]
fn filename_path_metadata_answer_does_not_force_content_repair_for_generic_contract() {
    let state = test_state();
    let loop_state = LoopState::new(1);
    let mut route = base_route_result();
    route.output_contract.semantic_kind = OutputSemanticKind::ArchiveUnpack;
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_hint = "rustclaw.service".to_string();
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "stat_paths",
                "paths": ["/home/guagua/rustclaw/rustclaw.service"]
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    assert!(!should_force_actionable_plan_repair(
        &state,
        Some(&route),
        &loop_state,
        &actions
    ));
}

#[test]
fn sqlite_table_names_route_rewrites_system_basic_action_alias_to_db_basic_list_tables() {
    let mut route = base_route_result();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::SqliteTableNamesOnly;
    route.output_contract.locator_hint = "/tmp/app.sqlite".to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "sqlite_table_names",
            "path": "/tmp/app.sqlite"
        }),
    }];

    let rewritten =
        rewrite_sqlite_table_listing_plan_to_db_basic(Some(&route), None, false, actions);

    match &rewritten[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "db_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("list_tables")
            );
            assert_eq!(
                args.get("db_path").and_then(|value| value.as_str()),
                Some("/tmp/app.sqlite")
            );
        }
        other => panic!("expected db_basic action, got {other:?}"),
    }
}

#[test]
fn sqlite_table_listing_route_rewrites_text_field_extract_to_db_basic_list_tables() {
    let mut route = base_route_result();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::SqliteTableListing;
    route.output_contract.locator_hint = "/tmp/app.sqlite".to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "extract_field",
            "path": "/tmp/app.sqlite",
            "field_path": "sqlite_master.name"
        }),
    }];

    let rewritten =
        rewrite_sqlite_table_listing_plan_to_db_basic(Some(&route), None, false, actions);

    match &rewritten[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "db_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("list_tables")
            );
            assert_eq!(
                args.get("db_path").and_then(|value| value.as_str()),
                Some("/tmp/app.sqlite")
            );
        }
        other => panic!("expected db_basic action, got {other:?}"),
    }
}

#[test]
fn sqlite_database_kind_judgment_rewrites_run_cmd_to_db_basic_list_tables() {
    let mut route = base_route_result();
    route.output_contract.semantic_kind = OutputSemanticKind::ArchiveUnpack;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::SqliteDatabaseKindJudgment;
    route.output_contract.locator_hint = "/tmp/db-basic-contract.sqlite".to_string();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "run_cmd",
                "command": "sqlite3 /tmp/db-basic-contract.sqlite \".tables\""
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
    ];

    let rewritten =
        rewrite_sqlite_table_listing_plan_to_db_basic(Some(&route), None, false, actions);

    match &rewritten[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "db_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("list_tables")
            );
            assert_eq!(
                args.get("db_path").and_then(|value| value.as_str()),
                Some("/tmp/db-basic-contract.sqlite")
            );
            assert!(args.get("sql").is_none());
        }
        other => panic!("expected db_basic action, got {other:?}"),
    }
    assert!(matches!(rewritten[1], AgentAction::SynthesizeAnswer { .. }));
}

#[test]
fn sqlite_table_listing_preserves_explicit_literal_run_cmd() {
    let mut route = base_route_result();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::SqliteTableListing;
    route.output_contract.locator_hint = "/tmp/app.sqlite".to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": "sqlite3 /tmp/app.sqlite '.tables'"}),
    }];

    let rewritten =
        rewrite_sqlite_table_listing_plan_to_db_basic(Some(&route), None, true, actions);

    match &rewritten[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "run_cmd");
            assert_eq!(
                args.get("command").and_then(Value::as_str),
                Some("sqlite3 /tmp/app.sqlite '.tables'")
            );
        }
        other => panic!("expected run_cmd action, got {other:?}"),
    }
}

#[test]
fn sqlite_schema_version_extract_field_rewrites_to_db_basic_pragma() {
    let mut route = base_route_result();
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/app.sqlite".to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "extract_field",
            "path": "/tmp/app.sqlite",
            "field_path": "schema_version"
        }),
    }];

    let rewritten =
        rewrite_sqlite_schema_version_plan_to_db_basic(Some(&route), None, false, actions);

    match &rewritten[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "db_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("schema_version")
            );
            assert_eq!(
                args.get("db_path").and_then(|value| value.as_str()),
                Some("/tmp/app.sqlite")
            );
            assert!(args.get("sql").is_none());
        }
        other => panic!("expected db_basic action, got {other:?}"),
    }
}

#[test]
fn sqlite_schema_version_extract_fields_rewrites_from_action_path() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({
            "action": "extract_fields",
            "path": "/tmp/app.db",
            "field_paths": ["schema_version"]
        }),
    }];

    let rewritten = rewrite_sqlite_schema_version_plan_to_db_basic(None, None, false, actions);

    match &rewritten[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "db_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("schema_version")
            );
            assert_eq!(
                args.get("db_path").and_then(|value| value.as_str()),
                Some("/tmp/app.db")
            );
            assert!(args.get("sql").is_none());
        }
        other => panic!("expected db_basic action, got {other:?}"),
    }
}

#[test]
fn sqlite_schema_version_route_rewrites_binary_text_read_to_db_basic_pragma() {
    let mut route = base_route_result();
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::SqliteSchemaVersion;
    route.output_contract.locator_hint = "/tmp/app.sqlite".to_string();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "read_range",
                "path": "/tmp/app.sqlite",
                "mode": "head",
                "n": 100
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
    ];

    let rewritten =
        rewrite_sqlite_schema_version_plan_to_db_basic(Some(&route), None, false, actions);

    match &rewritten[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "db_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("schema_version")
            );
            assert_eq!(
                args.get("db_path").and_then(|value| value.as_str()),
                Some("/tmp/app.sqlite")
            );
            assert!(args.get("sql").is_none());
        }
        other => panic!("expected db_basic action, got {other:?}"),
    }
    assert!(matches!(rewritten[1], AgentAction::SynthesizeAnswer { .. }));
}

#[test]
fn sqlite_count_query_rewrites_to_requested_schema_column_when_count_conflicts_with_column_intent()
{
    let tmp = TempDirGuard::new("sqlite_count_column_rewrite");
    let db_path = tmp.path.join("orders.sqlite");
    let conn = rusqlite::Connection::open(&db_path).expect("open sqlite");
    conn.execute(
        "CREATE TABLE orders (id INTEGER, user_id INTEGER, amount REAL, status TEXT)",
        [],
    )
    .expect("create table");
    let mut route = base_route_result();
    route.resolved_intent =
        "Read the amount of orders with status='pending' from the SQLite database".to_string();
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = db_path.display().to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "db_basic".to_string(),
        args: json!({
            "action": "sqlite_query",
            "db_path": db_path,
            "sql": "SELECT COUNT(*) FROM orders WHERE status='pending';"
        }),
    }];

    let rewritten = rewrite_sqlite_count_query_to_requested_schema_column(
        Some(&route),
        "Read the pending order amount.",
        None,
        actions,
    );

    let args = expect_planned_call(&rewritten[0], "db_basic", "sqlite_query");
    assert_eq!(
        args.get("sql").and_then(Value::as_str),
        Some(r#"SELECT "amount" FROM "orders" WHERE status='pending'"#)
    );
}

#[test]
fn sqlite_count_query_does_not_rewrite_scalar_count_contract() {
    let tmp = TempDirGuard::new("sqlite_count_contract_preserve");
    let db_path = tmp.path.join("users.sqlite");
    let conn = rusqlite::Connection::open(&db_path).expect("open sqlite");
    conn.execute("CREATE TABLE users (id INTEGER, name TEXT)", [])
        .expect("create table");
    let mut route = base_route_result();
    route.resolved_intent = "Count rows in the users table".to_string();
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
    route.output_contract.locator_hint = db_path.display().to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "db_basic".to_string(),
        args: json!({
            "action": "sqlite_query",
            "db_path": db_path,
            "sql": "SELECT COUNT(*) FROM users;"
        }),
    }];

    let rewritten = rewrite_sqlite_count_query_to_requested_schema_column(
        Some(&route),
        "How many users are stored?",
        None,
        actions,
    );

    let args = expect_planned_call(&rewritten[0], "db_basic", "sqlite_query");
    assert_eq!(
        args.get("sql").and_then(Value::as_str),
        Some("SELECT COUNT(*) FROM users;")
    );
}

#[test]
fn sqlite_table_probe_rewrites_to_requested_schema_value_query() {
    let tmp = TempDirGuard::new("sqlite_table_probe_value_rewrite");
    let db_path = tmp.path.join("orders.sqlite");
    let conn = rusqlite::Connection::open(&db_path).expect("open sqlite");
    conn.execute(
        "CREATE TABLE orders (id INTEGER, user_id INTEGER, amount REAL, status TEXT)",
        [],
    )
    .expect("create table");
    conn.execute(
        "INSERT INTO orders (id, user_id, amount, status) VALUES (1, 1, 7.5, 'pending')",
        [],
    )
    .expect("insert pending order");
    let mut route = base_route_result();
    route.resolved_intent =
        "Read the amount from the order table where status is pending".to_string();
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = db_path.display().to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "db_basic".to_string(),
        args: json!({
            "action": "list_tables",
            "db_path": db_path,
        }),
    }];

    let rewritten = rewrite_sqlite_table_probe_to_requested_schema_value(
        Some(&route),
        "Read the pending order amount.",
        None,
        actions,
    );

    let args = expect_planned_call(&rewritten[0], "db_basic", "sqlite_query");
    assert_eq!(
        args.get("sql").and_then(Value::as_str),
        Some(r#"SELECT "amount" FROM "orders" WHERE "status" = 'pending'"#)
    );
}

#[test]
fn sqlite_table_probe_keeps_table_listing_contract() {
    let tmp = TempDirGuard::new("sqlite_table_probe_listing_preserve");
    let db_path = tmp.path.join("orders.sqlite");
    let conn = rusqlite::Connection::open(&db_path).expect("open sqlite");
    conn.execute("CREATE TABLE orders (id INTEGER, status TEXT)", [])
        .expect("create table");
    let mut route = base_route_result();
    route.resolved_intent = "List the tables in the database".to_string();
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::SqliteTableNamesOnly;
    route.output_contract.locator_hint = db_path.display().to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "db_basic".to_string(),
        args: json!({
            "action": "list_tables",
            "db_path": db_path,
        }),
    }];

    let rewritten = rewrite_sqlite_table_probe_to_requested_schema_value(
        Some(&route),
        "List the tables in the database.",
        None,
        actions,
    );

    let args = expect_planned_call(&rewritten[0], "db_basic", "list_tables");
    assert!(args.get("sql").is_none());
}

#[test]
fn file_delivery_respond_only_gets_path_observation_before_file_token() {
    let tmp = TempDirGuard::new("file_delivery_observation");
    let file_path = tmp.path.join("service_notes.md");
    fs::write(&file_path, "notes\n").expect("write file");
    let state = test_state();
    let mut route = base_route_result();
    route.wants_file_delivery = true;
    route.output_contract.response_shape = OutputResponseShape::FileToken;
    route.output_contract.delivery_required = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = file_path.display().to_string();
    let token = format!("FILE:{}", file_path.display());
    let actions = vec![AgentAction::Respond { content: token }];

    let rewritten = replace_file_delivery_respond_only_with_path_observation(
        &state,
        Some(&route),
        &LoopState::default(),
        actions,
    );

    assert_eq!(rewritten.len(), 2);
    match &rewritten[0] {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            assert_eq!(skill, "fs_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("stat_paths")
            );
        }
        other => panic!("expected path observation, got {other:?}"),
    }
    assert!(matches!(rewritten[1], AgentAction::Respond { .. }));
}

#[test]
fn generated_file_write_delivery_appends_file_token() {
    let mut state = test_state();
    let tmp = TempDirGuard::new("generated_file_delivery_append_token");
    state.skill_rt.workspace_root = tmp.path.clone();
    let mut route = base_route_result();
    route.wants_file_delivery = true;
    route.output_contract.response_shape = OutputResponseShape::FileToken;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.semantic_kind = OutputSemanticKind::GeneratedFileDelivery;
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "write_text",
            "path": "tmp/对抗测试_笔记.txt",
            "content": "adversarial v1"
        }),
    }];

    let rewritten = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::default(),
        "create and deliver a file",
        None,
        actions,
    );

    assert_eq!(rewritten.len(), 2);
    assert!(planned_call_is(&rewritten[0], "fs_basic", "write_text"));
    let expected = format!("FILE:{}", tmp.path.join("tmp/对抗测试_笔记.txt").display());
    assert!(matches!(
        &rewritten[1],
        AgentAction::Respond { content } if content == &expected
    ));
}

#[test]
fn generated_file_write_delivery_replaces_non_file_terminal_respond() {
    let mut state = test_state();
    let tmp = TempDirGuard::new("generated_file_delivery_replace_respond");
    state.skill_rt.workspace_root = tmp.path.clone();
    let mut route = base_route_result();
    route.wants_file_delivery = true;
    route.output_contract.response_shape = OutputResponseShape::FileToken;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.semantic_kind = OutputSemanticKind::GeneratedFileDelivery;
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "write_text",
                "path": "tmp/note.txt",
                "content": "ok"
            }),
        },
        AgentAction::Respond {
            content: "created".to_string(),
        },
    ];

    let rewritten = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::default(),
        "create and deliver a file",
        None,
        actions,
    );

    assert_eq!(rewritten.len(), 2);
    let expected = format!("FILE:{}", tmp.path.join("tmp/note.txt").display());
    assert!(matches!(
        &rewritten[1],
        AgentAction::Respond { content } if content == &expected
    ));
}

#[tokio::test]
async fn parse_single_plan_preserves_fs_basic_write_text_tool_action() {
    let state = test_state_with_registry();
    let task = test_task();
    let raw = r#"{
      "steps": [
        {
          "type": "call_tool",
          "tool": "fs_basic",
          "args": {
            "action": "write_text",
            "path": "/tmp/path_note.txt",
            "content": "current path"
          }
        },
        {
          "type": "respond",
          "content": "/tmp/path_note.txt"
        }
      ]
    }"#;

    let actions = super::super::parse_single_plan_actions(raw, &state, &task)
        .await
        .expect("plan should parse");

    assert_eq!(actions.len(), 2);
    assert!(planned_call_is(&actions[0], "fs_basic", "write_text"));
    assert!(matches!(
        &actions[1],
        AgentAction::Respond { content } if content == "/tmp/path_note.txt"
    ));
}

#[test]
fn generated_file_path_report_keeps_command_write_and_path_respond_plan() {
    let root = TempDirGuard::new("generated_path_report_keep_write");
    let mut state = test_state_with_registry();
    state.skill_rt.workspace_root = root.path.clone();
    let target = root.path.join("path_note.txt");
    let request = "Run pwd, write a short line into path_note.txt, and reply with only the saved absolute file path.";
    let mut route = base_route_result();
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.delivery_required = false;
    route.output_contract.delivery_intent = OutputDeliveryIntent::None;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::GeneratedFilePathReport;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = target.display().to_string();
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
                "path": target.display().to_string(),
                "content": format!("current path: {}\n", root.path.display())
            }),
        },
        AgentAction::Respond {
            content: target.display().to_string(),
        },
    ];

    let normalized = super::super::normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &LoopState::new(1),
        request,
        Some(request),
        None,
        actions,
    );

    assert!(normalized.iter().any(|action| {
        matches!(
            action,
            AgentAction::CallTool { tool, args }
                if tool == "run_cmd"
                    && args.get("command").and_then(Value::as_str) == Some("pwd")
        )
    }));
    assert!(normalized
        .iter()
        .any(|action| { planned_call_is(action, "fs_basic", "write_text") }));
    assert!(matches!(
        normalized.last(),
        Some(AgentAction::Respond { content }) if content == &target.display().to_string()
    ));
}

#[test]
fn existing_file_delivery_stat_plan_appends_file_token() {
    let mut state = test_state();
    let tmp = TempDirGuard::new("existing_file_delivery_append_token");
    state.skill_rt.workspace_root = tmp.path.clone();
    let file_path = tmp.path.join("README.md");
    fs::write(&file_path, "readme\n").expect("write file");
    let mut route = base_route_result();
    route.wants_file_delivery = true;
    route.output_contract.response_shape = OutputResponseShape::FileToken;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "README.md".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::GeneratedFileDelivery;
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "path_batch_facts",
            "paths": ["README.md"],
            "include_missing": true,
        }),
    }];

    let rewritten = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::default(),
        "README.md",
        None,
        actions,
    );

    assert_eq!(rewritten.len(), 2);
    assert!(planned_call_is(
        &rewritten[0],
        "fs_basic",
        "path_batch_facts"
    ));
    let expected = format!("FILE:{}", file_path.display());
    assert!(matches!(
        &rewritten[1],
        AgentAction::Respond { content } if content == &expected
    ));
}

#[test]
fn content_summary_file_delivery_preserves_summary_without_terminal_file_token() {
    let mut state = test_state();
    let tmp = TempDirGuard::new("content_summary_existing_file_delivery");
    state.skill_rt.workspace_root = tmp.path.clone();
    let file_path = tmp.path.join("config.toml");
    fs::write(&file_path, "answer = true\n").expect("write file");
    let mut route = base_route_result();
    route.wants_file_delivery = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "config.toml".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptWithSummary;
    route.output_contract.requires_content_evidence = true;
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": "config.toml",
                "start_line": 1,
                "line_count": 20,
            }),
        },
        AgentAction::Respond {
            content: "observed summary".to_string(),
        },
    ];

    let rewritten = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::default(),
        "summarize and deliver config.toml",
        None,
        actions,
    );

    assert_eq!(rewritten.len(), 2);
    assert!(planned_call_is(
        &rewritten[0],
        "fs_basic",
        "read_text_range"
    ));
    assert!(matches!(
        &rewritten[1],
        AgentAction::Respond { content } if content == "observed summary"
    ));
}

#[test]
fn mixed_file_token_prose_after_existing_file_read_rewrites_to_synthesize() {
    let mut state = test_state();
    let tmp = TempDirGuard::new("mixed_file_token_prose_existing_file_delivery");
    state.skill_rt.workspace_root = tmp.path.clone();
    let file_path = tmp.path.join("config.toml");
    fs::write(&file_path, "answer = true\n").expect("write file");
    let mut route = base_route_result();
    route.wants_file_delivery = true;
    route.output_contract.response_shape = OutputResponseShape::FileToken;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = file_path.display().to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::GeneratedFileDelivery;
    route.output_contract.requires_content_evidence = true;
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": file_path.display().to_string(),
            }),
        },
        AgentAction::Respond {
            content: format!("FILE:{}\nobserved summary", file_path.display()),
        },
    ];

    let rewritten = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::default(),
        "deliver config and summarize",
        None,
        actions,
    );

    assert_eq!(rewritten.len(), 3);
    assert!(planned_call_is(
        &rewritten[0],
        "fs_basic",
        "read_text_range"
    ));
    assert!(matches!(
        &rewritten[1],
        AgentAction::SynthesizeAnswer { evidence_refs } if evidence_refs == &vec!["last_output".to_string()]
    ));
    assert!(!rewritten.iter().any(|action| matches!(
        action,
        AgentAction::Respond { content }
            if content.contains("FILE:") && content.contains("{{last_output}}")
    )));
}

#[test]
fn mixed_file_token_prose_after_prior_read_rewrites_to_synthesize() {
    use crate::executor::{StepExecutionResult, StepExecutionStatus};

    let mut state = test_state();
    let tmp = TempDirGuard::new("mixed_file_token_prose_prior_read");
    state.skill_rt.workspace_root = tmp.path.clone();
    let file_path = tmp.path.join("config.toml");
    fs::write(&file_path, "answer = true\n").expect("write file");
    let mut route = base_route_result();
    route.wants_file_delivery = true;
    route.output_contract.response_shape = OutputResponseShape::FileToken;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = file_path.display().to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::GeneratedFileDelivery;
    route.output_contract.requires_content_evidence = true;
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            json!({
                "extra": {
                    "action": "read_range",
                    "path": file_path.display().to_string(),
                    "resolved_path": file_path.display().to_string(),
                    "excerpt": "1|answer = true"
                }
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });
    let actions = vec![AgentAction::Respond {
        content: format!("FILE:{}\nobserved summary", file_path.display()),
    }];

    let rewritten = normalize_planned_actions(
        &state,
        Some(&route),
        &loop_state,
        "deliver config and summarize",
        None,
        actions,
    );

    assert_eq!(rewritten.len(), 2);
    assert!(matches!(
        &rewritten[0],
        AgentAction::SynthesizeAnswer { evidence_refs } if evidence_refs == &vec!["last_output".to_string()]
    ));
    assert!(matches!(
        &rewritten[1],
        AgentAction::Respond { content } if content == &format!("FILE:{}", file_path.display())
    ));
}

#[test]
fn mixed_file_token_placeholder_for_content_delivery_rewrites_to_synthesize() {
    let mut state = test_state();
    let tmp = TempDirGuard::new("mixed_file_token_placeholder_content_delivery");
    state.skill_rt.workspace_root = tmp.path.clone();
    let file_path = tmp.path.join("config.toml");
    fs::write(&file_path, "answer = true\n").expect("write file");
    let mut route = base_route_result();
    route.wants_file_delivery = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = file_path.display().to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptWithSummary;
    route.output_contract.requires_content_evidence = true;
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": file_path.display().to_string(),
            }),
        },
        AgentAction::Respond {
            content: format!("FILE:{}\n{{{{last_output}}}}", file_path.display()),
        },
    ];

    let rewritten = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::default(),
        "deliver config and summarize",
        None,
        actions,
    );

    assert_eq!(rewritten.len(), 3);
    assert!(planned_call_is(
        &rewritten[0],
        "fs_basic",
        "read_text_range"
    ));
    assert!(matches!(
        &rewritten[1],
        AgentAction::SynthesizeAnswer { evidence_refs } if evidence_refs == &vec!["last_output".to_string()]
    ));
    assert!(!rewritten.iter().any(|action| matches!(
        action,
        AgentAction::Respond { content }
            if content.contains("FILE:") && content.contains("{{last_output}}")
    )));
}

use super::*;

#[test]
fn file_delivery_terminal_token_is_not_rewritten_to_content_synthesis() {
    let mut route = base_route_result();
    route.wants_file_delivery = true;
    route.output_contract.response_shape = OutputResponseShape::FileToken;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/LICENSE.zh-CN.md".to_string();
    route.output_contract.requires_content_evidence = true;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: json!({"path":"/tmp/LICENSE.zh-CN.md"}),
        },
        AgentAction::Respond {
            content: "FILE:/tmp/LICENSE.zh-CN.md".to_string(),
        },
    ];

    let rewritten = rewrite_pre_observation_concrete_respond_to_placeholder(
        &test_state(),
        Some(&route),
        &LoopState::default(),
        actions,
    );

    assert_eq!(rewritten.len(), 2);
    assert!(matches!(rewritten[0], AgentAction::CallSkill { .. }));
    assert!(matches!(
        &rewritten[1],
        AgentAction::Respond { content } if content == "FILE:/tmp/LICENSE.zh-CN.md"
    ));
}

#[test]
fn file_token_respond_survives_even_when_delivery_contract_is_missing() {
    let mut route = base_route_result();
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.delivery_required = false;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/LICENSE.zh-CN.md".to_string();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: json!({"path":"/tmp/LICENSE.zh-CN.md"}),
        },
        AgentAction::Respond {
            content: "FILE:/tmp/LICENSE.zh-CN.md".to_string(),
        },
    ];

    let rewritten = rewrite_pre_observation_concrete_respond_to_placeholder(
        &test_state(),
        Some(&route),
        &LoopState::default(),
        actions,
    );

    assert_eq!(rewritten.len(), 2);
    assert!(matches!(
        &rewritten[1],
        AgentAction::Respond { content } if content == "FILE:/tmp/LICENSE.zh-CN.md"
    ));
}

#[test]
fn archive_unpack_route_rewrites_run_cmd_unzip_to_archive_basic() {
    let mut route = base_route_result();
    route.route_reason = "capability_ref=archive.unpack".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::ArchiveUnpack;
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/bundle.zip | /tmp/out".to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({
            "command": "unzip \"/tmp/bundle.zip\" -d \"/tmp/out\""
        }),
    }];

    let rewritten = rewrite_archive_unpack_run_cmd_to_archive_basic(Some(&route), false, actions);

    match &rewritten[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "archive_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("unpack")
            );
            assert_eq!(
                args.get("archive").and_then(|value| value.as_str()),
                Some("/tmp/bundle.zip")
            );
            assert_eq!(
                args.get("dest").and_then(|value| value.as_str()),
                Some("/tmp/out")
            );
        }
        other => panic!("expected archive_basic action, got {other:?}"),
    }
}

#[test]
fn archive_unpack_route_rewrites_archive_read_plan_to_unpack() {
    let mut route = base_route_result();
    route.route_reason = "capability_ref=archive.unpack".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::ArchiveUnpack;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/bundle.zip | /tmp/out".to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "archive_basic".to_string(),
        args: json!({
            "action": "read",
            "archive": "/tmp/bundle.zip",
            "member": "/tmp/out",
        }),
    }];

    let rewritten = rewrite_archive_unpack_run_cmd_to_archive_basic(Some(&route), false, actions);

    match &rewritten[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "archive_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("unpack")
            );
            assert_eq!(
                args.get("archive").and_then(|value| value.as_str()),
                Some("/tmp/bundle.zip")
            );
            assert_eq!(
                args.get("dest").and_then(|value| value.as_str()),
                Some("/tmp/out")
            );
        }
        other => panic!("expected archive_basic action, got {other:?}"),
    }
}

#[test]
fn content_excerpt_archive_member_read_is_not_rewritten_to_unpack() {
    let mut route = base_route_result();
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip | notes.txt".to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "archive_basic".to_string(),
        args: json!({
            "action": "read",
            "archive": "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip",
            "member": "notes.txt",
        }),
    }];

    let rewritten = rewrite_archive_unpack_run_cmd_to_archive_basic(Some(&route), false, actions);

    let args = expect_planned_call(&rewritten[0], "archive_basic", "read");
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
fn archive_unpack_capability_ref_allows_planner_supplied_args_with_semantic_kind() {
    let mut route = base_route_result();
    route.route_reason = "capability_ref=archive.unpack".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::ArchiveUnpack;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip | tmp/contract_matrix_unpacked"
            .to_string();

    let policy = crate::evidence_policy::capability_ref_action_policy_for_route(
        Some(&route),
        "archive_basic",
        &json!({
            "action": "unpack",
            "archive": "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip",
            "dest": "tmp/contract_matrix_unpacked",
        }),
    )
    .expect("archive.unpack capability ref should expose archive_basic.unpack");

    assert!(policy.is_allowed(), "{policy:?}");
    assert!(policy.action_matches_preferred(), "{policy:?}");
}

#[test]
fn archive_unpack_capability_ref_allows_planner_supplied_args_without_semantic_kind() {
    let mut route = base_route_result();
    route.route_reason = "capability_ref=archive.unpack".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip | tmp/contract_matrix_unpacked"
            .to_string();

    let policy = crate::evidence_policy::capability_ref_action_policy_for_route(
        Some(&route),
        "archive_basic",
        &json!({
            "action": "unpack",
            "archive": "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip",
            "dest": "tmp/contract_matrix_unpacked",
        }),
    )
    .expect("archive.unpack capability ref should work without ArchiveUnpack semantic kind");

    assert!(policy.is_allowed(), "{policy:?}");
    assert!(policy.action_matches_preferred(), "{policy:?}");
}

#[test]
fn archive_pack_capability_ref_allows_planner_supplied_args_with_semantic_kind() {
    let mut route = base_route_result();
    route.route_reason = "capability_ref=archive.pack".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::ArchivePack;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/skill_calls | tmp/nl_archive_case_en.zip".to_string();

    let policy = crate::evidence_policy::capability_ref_action_policy_for_route(
        Some(&route),
        "archive_basic",
        &json!({
            "action": "pack",
            "source": "scripts/skill_calls",
            "archive": "tmp/nl_archive_case_en.zip",
            "format": "zip",
        }),
    )
    .expect("archive.pack capability ref should expose archive_basic.pack");

    assert!(policy.is_allowed(), "{policy:?}");
    assert!(policy.action_matches_preferred(), "{policy:?}");
}

#[test]
fn archive_pack_capability_ref_allows_planner_supplied_args_without_semantic_kind() {
    let mut route = base_route_result();
    route.route_reason = "capability_ref=archive.pack".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/skill_calls | tmp/nl_archive_case_en.zip".to_string();

    let policy = crate::evidence_policy::capability_ref_action_policy_for_route(
        Some(&route),
        "archive_basic",
        &json!({
            "action": "pack",
            "source": "scripts/skill_calls",
            "archive": "tmp/nl_archive_case_en.zip",
            "format": "zip",
        }),
    )
    .expect("archive.pack capability ref should work without ArchivePack semantic kind");

    assert!(policy.is_allowed(), "{policy:?}");
    assert!(policy.action_matches_preferred(), "{policy:?}");
}

#[test]
fn filesystem_mutation_archive_target_capability_ref_allows_pack_policy() {
    let root = TempDirGuard::new("filesystem_mutation_archive_pack");
    fs::create_dir_all(root.path.join("scripts/skill_calls")).expect("create source dir");
    fs::create_dir_all(root.path.join("tmp")).expect("create tmp dir");
    let archive_path = root.path.join("tmp/nl_archive_case_en.zip");
    let archive_path_text = archive_path.display().to_string();
    let mut route = base_route_result();
    route.route_reason = "capability_ref=archive.pack".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::FilesystemMutationResult;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = archive_path_text.clone();
    route.resolved_intent = format!(
        "Zip the scripts/skill_calls directory into {archive_path_text} and report success"
    );

    let policy = crate::evidence_policy::capability_ref_action_policy_for_route(
        Some(&route),
        "archive_basic",
        &json!({
            "action": "pack",
            "source": "scripts/skill_calls",
            "archive": archive_path_text,
            "format": "zip",
        }),
    )
    .expect("archive.pack capability ref should allow planner-supplied filesystem mutation args");

    assert!(policy.is_allowed(), "{policy:?}");
    assert!(policy.action_matches_preferred(), "{policy:?}");
}

#[test]
fn archive_unpack_preserves_explicit_literal_run_cmd() {
    let mut route = base_route_result();
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/input.zip | /tmp/out".to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": "unzip /tmp/input.zip -d /tmp/out"}),
    }];

    let rewritten = rewrite_archive_unpack_run_cmd_to_archive_basic(Some(&route), true, actions);

    match &rewritten[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "run_cmd");
            assert_eq!(
                args.get("command").and_then(Value::as_str),
                Some("unzip /tmp/input.zip -d /tmp/out")
            );
        }
        other => panic!("expected run_cmd action, got {other:?}"),
    }
}

#[test]
fn archive_pack_route_rewrites_probe_only_plan_to_archive_basic() {
    let mut route = base_route_result();
    route.route_reason = "capability_ref=archive.pack".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::ArchivePack;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/skill_calls | tmp/nl_archive_case_en.zip".to_string();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({
                "action": "path_batch_facts",
                "paths": [
                    "/home/guagua/rustclaw/scripts/skill_calls",
                    "/home/guagua/rustclaw/tmp/nl_archive_case_en.zip"
                ]
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "Unable to create the zip archive.".to_string(),
        },
    ];

    let rewritten = rewrite_archive_pack_plan_to_archive_basic(
        Some(&route),
        &LoopState::new(2),
        false,
        actions,
    );

    assert_eq!(rewritten.len(), 2);
    match &rewritten[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "archive_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("pack")
            );
            assert_eq!(
                args.get("source").and_then(|value| value.as_str()),
                Some("scripts/skill_calls")
            );
            assert_eq!(
                args.get("archive").and_then(|value| value.as_str()),
                Some("tmp/nl_archive_case_en.zip")
            );
            assert_eq!(
                args.get("format").and_then(|value| value.as_str()),
                Some("zip")
            );
        }
        other => panic!("expected archive_basic action, got {other:?}"),
    }
    assert!(matches!(rewritten[1], AgentAction::SynthesizeAnswer { .. }));
}

#[test]
fn archive_pack_route_rewrites_archive_list_plan_to_pack() {
    let mut route = base_route_result();
    route.route_reason = "capability_ref=archive.pack".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::ArchivePack;
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/skill_calls | tmp/nl_archive_case_en.zip".to_string();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "archive_basic".to_string(),
            args: json!({
                "action": "list",
                "archive": "tmp/nl_archive_case_en.zip",
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let rewritten = rewrite_archive_pack_plan_to_archive_basic(
        Some(&route),
        &LoopState::new(2),
        false,
        actions,
    );

    assert_eq!(rewritten.len(), 2);
    match &rewritten[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "archive_basic");
            assert_eq!(args.get("action").and_then(Value::as_str), Some("pack"));
            assert_eq!(
                args.get("source").and_then(Value::as_str),
                Some("scripts/skill_calls")
            );
            assert_eq!(
                args.get("archive").and_then(Value::as_str),
                Some("tmp/nl_archive_case_en.zip")
            );
        }
        other => panic!("expected archive_basic pack action, got {other:?}"),
    }
    assert!(matches!(rewritten[1], AgentAction::SynthesizeAnswer { .. }));
}

#[test]
fn archive_pack_route_preserves_post_pack_list_and_cleanup_plan() {
    let mut route = base_route_result();
    route.route_reason = "capability_ref=archive.pack".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::ArchivePack;
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/skill_calls | tmp/nl_archive_case_en.zip".to_string();
    let mut loop_state = LoopState::new(3);
    loop_state
        .executed_step_results
        .push(crate::executor::StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "archive_basic".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some(
                r#"{"extra":{"action":"pack","archive":"/home/guagua/rustclaw/tmp/nl_archive_case_en.zip","format":"zip","source":"scripts/skill_calls"},"text":"archive_path=/home/guagua/rustclaw/tmp/nl_archive_case_en.zip"}"#
                    .to_string(),
            ),
            error: None,
            started_at: 1,
            finished_at: 2,
        });
    let actions = vec![
        AgentAction::CallSkill {
            skill: "archive_basic".to_string(),
            args: json!({
                "action": "list",
                "archive": "tmp/nl_archive_case_en.zip",
            }),
        },
        AgentAction::CallSkill {
            skill: "fs_basic".to_string(),
            args: json!({
                "action": "remove_path",
                "path": "tmp/nl_archive_case_en.zip",
            }),
        },
        AgentAction::Respond {
            content: "created_archive_path=tmp/nl_archive_case_en.zip".to_string(),
        },
    ];

    let rewritten =
        rewrite_archive_pack_plan_to_archive_basic(Some(&route), &loop_state, false, actions);

    assert_eq!(rewritten.len(), 3);
    assert!(matches!(
        &rewritten[0],
        AgentAction::CallSkill { skill, args }
            if skill == "archive_basic"
                && args.get("action").and_then(Value::as_str) == Some("list")
    ));
    assert!(matches!(
        &rewritten[1],
        AgentAction::CallSkill { skill, args }
            if skill == "fs_basic"
                && args.get("action").and_then(Value::as_str) == Some("remove_path")
    ));
}

#[test]
fn archive_pack_preserves_explicit_literal_run_cmd() {
    let mut route = base_route_result();
    route.output_contract.semantic_kind = OutputSemanticKind::ArchivePack;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/source | /tmp/source.tgz".to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command": "tar -czf /tmp/source.tgz /tmp/source"}),
    }];

    let rewritten =
        rewrite_archive_pack_plan_to_archive_basic(Some(&route), &LoopState::new(2), true, actions);

    match &rewritten[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "run_cmd");
            assert_eq!(
                args.get("command").and_then(Value::as_str),
                Some("tar -czf /tmp/source.tgz /tmp/source")
            );
        }
        other => panic!("expected run_cmd action, got {other:?}"),
    }
}

#[test]
fn archive_basic_pack_alias_args_normalize_to_contract() {
    let mut route = base_route_result();
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/skill_calls -> tmp/nl_archive_case_en.zip".to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "archive_basic".to_string(),
        args: json!({
            "action": "pack",
            "source_path": "/home/guagua/rustclaw/scripts/skill_calls",
            "archive_path": "/home/guagua/rustclaw/tmp/nl_archive_case_en.zip",
        }),
    }];

    let normalized = normalize_archive_basic_schema_aliases(Some(&route), actions);

    match &normalized[0] {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "archive_basic");
            assert_eq!(
                args.get("source").and_then(Value::as_str),
                Some("/home/guagua/rustclaw/scripts/skill_calls")
            );
            assert_eq!(
                args.get("archive").and_then(Value::as_str),
                Some("/home/guagua/rustclaw/tmp/nl_archive_case_en.zip")
            );
            assert_eq!(args.get("format").and_then(Value::as_str), Some("zip"));
            assert!(args.get("source_path").is_none());
            assert!(args.get("archive_path").is_none());
        }
        other => panic!("expected archive_basic action, got {other:?}"),
    }
}

#[test]
fn config_edit_action_ref_tool_call_rewrites_to_real_tool_and_value_arg() {
    let mut route = base_route_result();
    route.output_contract.semantic_kind = OutputSemanticKind::ConfigMutation;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();
    route.route_reason =
        "capability_ref=config.plan_change field_path=skills.skill_switches.config_edit_nl_plan value=true"
            .to_string();
    let actions = vec![AgentAction::CallTool {
        tool: "config_edit.plan_config_change".to_string(),
        args: json!({
            "path": "configs/config.toml",
            "field_path": "skills.skill_switches.config_edit_nl_plan",
            "new_value": true,
            "plan_only": true,
        }),
    }];

    let rewritten = normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::default(),
        "configs/config.toml wrong.field = false",
        Some("configs/config.toml"),
        actions,
    );

    assert_eq!(rewritten.len(), 3);
    let args = expect_planned_call(&rewritten[0], "config_edit", "plan_config_change");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some("configs/config.toml")
    );
    assert_eq!(
        args.get("field_path").and_then(Value::as_str),
        Some("skills.skill_switches.config_edit_nl_plan")
    );
    assert_eq!(args.get("value").and_then(Value::as_bool), Some(true));
    assert!(args.get("new_value").is_none());
}

#[test]
fn config_mutation_read_plus_plan_collapses_to_single_config_edit_plan() {
    let mut route = base_route_result();
    route.output_contract.semantic_kind = OutputSemanticKind::ConfigMutation;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();
    route.route_reason =
        "capability_ref=config.plan_change field_path=skills.skill_switches.config_edit_nl_plan value=true"
            .to_string();
    let actions = vec![
        AgentAction::CallTool {
            tool: "config_basic.read_field".to_string(),
            args: json!({
                "path": "configs/config.toml",
                "field_path": "skills.skill_switches.config_edit_nl_plan",
            }),
        },
        AgentAction::CallTool {
            tool: "config_edit.plan_config_change".to_string(),
            args: json!({
                "path": "configs/config.toml",
                "field_path": "skills.skill_switches.config_edit_nl_plan",
                "new_value": true,
                "mode": "plan_only",
            }),
        },
    ];

    let rewritten = normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::default(),
        "configs/config.toml wrong.field = false",
        Some("configs/config.toml"),
        actions,
    );

    assert_eq!(rewritten.len(), 3);
    let args = expect_planned_call(&rewritten[0], "config_edit", "plan_config_change");
    assert_eq!(
        args.get("field_path").and_then(Value::as_str),
        Some("skills.skill_switches.config_edit_nl_plan")
    );
    assert_eq!(args.get("value").and_then(Value::as_bool), Some(true));
}

#[test]
fn config_change_preview_read_plan_does_not_infer_mutation_from_user_text() {
    let mut route = base_route_result();
    route.output_contract.semantic_kind = OutputSemanticKind::ConfigMutation;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();
    let actions = vec![
        AgentAction::CallTool {
            tool: "config_basic".to_string(),
            args: json!({
                "action": "read_field",
                "path": "configs/config.toml",
                "field_path": "skills.skill_switches",
                "format": "toml",
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
    ];

    let rewritten = rewrite_config_change_preview_to_config_edit_plan(
        Some(&route),
        "只生成变更计划，不要实际修改：把 configs/config.toml 里的 wrong.field 设置为 false",
        Some("configs/config.toml"),
        actions,
    );

    assert_eq!(rewritten.len(), 2);
    let args = expect_planned_call(&rewritten[0], "config_basic", "read_field");
    assert_eq!(
        args.get("field_path").and_then(Value::as_str),
        Some("skills.skill_switches")
    );
}

#[test]
fn config_change_preview_guard_plan_rewrites_to_config_edit_plan() {
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::ConfigMutation;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();
    route.route_reason =
        "capability_ref=config.plan_change field_path=skills.skill_switches.config_edit_nl_plan value=true"
            .to_string();
    let actions = vec![
        AgentAction::CallTool {
            tool: "config_basic".to_string(),
            args: json!({
                "action": "guard_rustclaw_config",
                "path": "configs/config.toml",
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
    ];

    let rewritten = rewrite_config_change_preview_to_config_edit_plan(
        Some(&route),
        "configs/config.toml wrong.field = false",
        Some("configs/config.toml"),
        actions,
    );

    assert_eq!(rewritten.len(), 1);
    let args = expect_planned_call(&rewritten[0], "config_edit", "plan_config_change");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some("configs/config.toml")
    );
    assert_eq!(
        args.get("field_path").and_then(Value::as_str),
        Some("skills.skill_switches.config_edit_nl_plan")
    );
    assert_eq!(args.get("value").and_then(Value::as_bool), Some(true));
}

#[test]
fn config_change_capability_ref_rewrites_preview_without_semantic_kind() {
    let mut route = base_route_result();
    route.route_reason =
        "capability_ref=config.plan_change field_path=skills.skill_switches.config_edit_nl_plan value=true"
            .to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();
    let actions = vec![
        AgentAction::CallTool {
            tool: "config_basic".to_string(),
            args: json!({
                "action": "read_field",
                "path": "configs/config.toml",
                "field_path": "skills.skill_switches",
                "format": "toml",
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
    ];

    let rewritten = rewrite_config_change_preview_to_config_edit_plan(
        Some(&route),
        "configs/config.toml wrong.field = false",
        Some("configs/config.toml"),
        actions,
    );

    assert_eq!(rewritten.len(), 1);
    let args = expect_planned_call(&rewritten[0], "config_edit", "plan_config_change");
    assert_eq!(
        args.get("field_path").and_then(Value::as_str),
        Some("skills.skill_switches.config_edit_nl_plan")
    );
    assert_eq!(args.get("value").and_then(Value::as_bool), Some(true));
}

#[test]
fn config_plan_only_capability_ref_rewrites_without_semantic_kind() {
    let mut route = base_route_result();
    route.route_reason =
        "capability_ref=config.plan_change field_path=skills.skill_switches.config_edit_nl_plan value=true"
            .to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();

    let rewritten = rewrite_config_mutation_plan_only_to_config_edit_plan(
        Some(&route),
        &LoopState::new(1),
        "configs/config.toml wrong.field = false",
        Some("configs/config.toml"),
        Vec::new(),
    );

    assert_eq!(rewritten.len(), 1);
    let args = expect_planned_call(&rewritten[0], "config_edit", "plan_config_change");
    assert_eq!(
        args.get("field_path").and_then(Value::as_str),
        Some("skills.skill_switches.config_edit_nl_plan")
    );
    assert_eq!(args.get("value").and_then(Value::as_bool), Some(true));
}

#[test]
fn config_mutation_recipe_readback_only_rewrites_to_closed_loop() {
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::ConfigMutation;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "run/nl_eval_tmp/config_edit_smoke/config.toml".to_string();
    route.route_reason =
        "capability_ref=config.apply_change field_path=skills.skill_switches.config_edit_nl_smoke value=true"
            .to_string();
    let mut loop_state = LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
        crate::execution_recipe::ExecutionRecipeSpec {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            profile: crate::execution_recipe::ExecutionRecipeProfile::ConfigChange,
            target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::CurrentRepo,
            inspect_first: true,
            validation_required: true,
            max_repairs: 2,
        },
    );
    let actions = vec![
        AgentAction::CallTool {
            tool: "config_edit".to_string(),
            args: json!({
                "action": "read_back",
                "path": "run/nl_eval_tmp/config_edit_smoke/config.toml",
                "field_path": "skills.skill_switches.config_edit_nl_smoke",
                "format": "toml",
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let rewritten = rewrite_config_mutation_to_config_edit_closed_loop(
        Some(&route),
        &loop_state,
        "run/nl_eval_tmp/config_edit_smoke/config.toml wrong.field = false",
        Some("run/nl_eval_tmp/config_edit_smoke/config.toml"),
        actions,
    );

    assert_eq!(rewritten.len(), 6);
    let plan_args = expect_planned_call(&rewritten[0], "config_edit", "plan_config_change");
    assert_eq!(
        plan_args.get("path").and_then(Value::as_str),
        Some("run/nl_eval_tmp/config_edit_smoke/config.toml")
    );
    assert_eq!(
        plan_args.get("field_path").and_then(Value::as_str),
        Some("skills.skill_switches.config_edit_nl_smoke")
    );
    assert_eq!(plan_args.get("value").and_then(Value::as_bool), Some(true));
    assert_eq!(
        plan_args.get("format").and_then(Value::as_str),
        Some("toml")
    );

    let apply_args = expect_planned_call(&rewritten[1], "config_edit", "apply_config_change");
    assert_eq!(
        apply_args.get("field_path").and_then(Value::as_str),
        Some("skills.skill_switches.config_edit_nl_smoke")
    );
    assert_eq!(apply_args.get("value").and_then(Value::as_bool), Some(true));
    assert_eq!(
        apply_args.get("operation").and_then(Value::as_str),
        Some("set")
    );
    let validate_args = expect_planned_call(&rewritten[2], "config_edit", "validate_config");
    assert_eq!(
        validate_args.get("format").and_then(Value::as_str),
        Some("toml")
    );
    let read_back_args = expect_planned_call(&rewritten[3], "config_edit", "read_back");
    assert_eq!(
        read_back_args.get("field_path").and_then(Value::as_str),
        Some("skills.skill_switches.config_edit_nl_smoke")
    );
    assert!(matches!(
        &rewritten[4],
        AgentAction::SynthesizeAnswer { evidence_refs }
            if evidence_refs == &vec![
                "step_1".to_string(),
                "step_2".to_string(),
                "step_3".to_string(),
                "step_4".to_string()
            ]
    ));
}

#[test]
fn config_change_capability_ref_rewrites_to_closed_loop_without_semantic_kind() {
    let mut route = base_route_result();
    route.route_reason =
        "capability_ref=config.apply_change field_path=skills.skill_switches.config_edit_nl_smoke value=true"
            .to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "run/nl_eval_tmp/config_edit_smoke/config.toml".to_string();
    let mut loop_state = LoopState::new(2);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
        crate::execution_recipe::ExecutionRecipeSpec {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            profile: crate::execution_recipe::ExecutionRecipeProfile::ConfigChange,
            target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::CurrentRepo,
            inspect_first: true,
            validation_required: true,
            max_repairs: 2,
        },
    );
    let actions = vec![AgentAction::CallTool {
        tool: "config_edit".to_string(),
        args: json!({
            "action": "read_back",
            "path": "run/nl_eval_tmp/config_edit_smoke/config.toml",
            "field_path": "skills.skill_switches.config_edit_nl_smoke",
            "format": "toml",
        }),
    }];

    let rewritten = rewrite_config_mutation_to_config_edit_closed_loop(
        Some(&route),
        &loop_state,
        "run/nl_eval_tmp/config_edit_smoke/config.toml wrong.field = false",
        Some("run/nl_eval_tmp/config_edit_smoke/config.toml"),
        actions,
    );

    assert_eq!(rewritten.len(), 6);
    expect_planned_call(&rewritten[0], "config_edit", "plan_config_change");
    expect_planned_call(&rewritten[1], "config_edit", "apply_config_change");
    expect_planned_call(&rewritten[2], "config_edit", "validate_config");
    expect_planned_call(&rewritten[3], "config_edit", "read_back");
}

#[test]
fn config_mutation_planned_fs_write_rewrites_to_config_edit_closed_loop_without_recipe() {
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::ConfigMutation;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "run/nl_eval_tmp/config_edit_smoke/config.toml".to_string();
    route.route_reason =
        "capability_ref=config.apply_change field_path=skills.skill_switches.config_edit_nl_smoke value=true"
            .to_string();
    let loop_state = LoopState::new(2);
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_range",
                "path": "run/nl_eval_tmp/config_edit_smoke/config.toml",
            }),
        },
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "write_text",
                "path": "run/nl_eval_tmp/config_edit_smoke/config.toml",
                "content": "[skills]\n\n[skills.skill_switches]\nconfig_edit_nl_smoke = true\n",
            }),
        },
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_range",
                "path": "run/nl_eval_tmp/config_edit_smoke/config.toml",
            }),
        },
    ];

    let rewritten = rewrite_config_mutation_to_config_edit_closed_loop(
        Some(&route),
        &loop_state,
        "run/nl_eval_tmp/config_edit_smoke/config.toml wrong.field = false",
        Some("run/nl_eval_tmp/config_edit_smoke/config.toml"),
        actions,
    );

    assert_eq!(rewritten.len(), 6);
    let plan_args = expect_planned_call(&rewritten[0], "config_edit", "plan_config_change");
    assert_eq!(
        plan_args.get("field_path").and_then(Value::as_str),
        Some("skills.skill_switches.config_edit_nl_smoke")
    );
    assert_eq!(plan_args.get("value").and_then(Value::as_bool), Some(true));
    expect_planned_call(&rewritten[1], "config_edit", "apply_config_change");
    expect_planned_call(&rewritten[2], "config_edit", "validate_config");
    expect_planned_call(&rewritten[3], "config_edit", "read_back");
}

#[test]
fn config_mutation_without_recipe_does_not_upgrade_readback_to_apply() {
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::ConfigMutation;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();
    let loop_state = LoopState::new(2);
    let actions = vec![AgentAction::CallTool {
        tool: "config_edit".to_string(),
        args: json!({
            "action": "read_back",
            "path": "configs/config.toml",
            "field_path": "skills.skill_switches.config_edit_nl_plan",
            "format": "toml",
        }),
    }];

    let rewritten = rewrite_config_mutation_to_config_edit_closed_loop(
        Some(&route),
        &loop_state,
        "configs/config.toml skills.skill_switches.config_edit_nl_plan = true",
        Some("configs/config.toml"),
        actions,
    );

    assert_eq!(rewritten.len(), 1);
    expect_planned_call(&rewritten[0], "config_edit", "read_back");
}

#[test]
fn unrequested_config_edit_is_stripped_from_text_rewrite_followup() {
    let state = test_state_with_enabled_skills(&["config_edit", "synthesize_answer"]);
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.resolved_intent = "rewrite_active_text_style_only".to_string();
    route.route_reason = "style_transform_without_config_anchor".to_string();
    let loop_state = LoopState::new(2);
    let actions = vec![
        AgentAction::CallTool {
            tool: "config_edit".to_string(),
            args: json!({
                "action": "plan_config_change",
                "path": "configs/config.toml",
                "field_path": "build-all.sh",
                "value": 1
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["step_1".to_string()],
        },
        AgentAction::Respond {
            content: "rewritten_text_body".to_string(),
        },
    ];

    let normalized = normalize_planned_actions_with_original(
            &state,
            Some(&route),
            &loop_state,
            "active_task_id=summary\nprevious_output=configs/config.toml build-all.sh 1\ncurrent_instruction=rewrite_style_only",
            Some("rewrite_style_only"),
            None,
            actions,
        );

    assert!(
        !normalized.iter().any(action_targets_config_edit),
        "normalized actions: {normalized:?}"
    );
    assert!(matches!(
        normalized.as_slice(),
        [AgentAction::Respond { content }] if content == "rewritten_text_body"
    ));
}

#[test]
fn requested_config_edit_plan_is_preserved_by_structural_anchors() {
    let state = test_state_with_enabled_skills(&["config_edit"]);
    let route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Strict,
    );
    let loop_state = LoopState::new(2);
    let actions = vec![AgentAction::CallTool {
        tool: "config_edit".to_string(),
        args: json!({
            "action": "plan_config_change",
            "path": "configs/config.toml",
            "field_path": "server.port",
            "value": 8787
        }),
    }];

    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &loop_state,
        "configs/config.toml server.port 8787",
        Some("configs/config.toml server.port 8787"),
        None,
        actions,
    );

    assert!(
        normalized.iter().any(action_targets_config_edit),
        "normalized actions: {normalized:?}"
    );
}

#[test]
fn direct_answer_config_contract_preserves_config_edit_plan() {
    let state = test_state_with_enabled_skills(&["config_edit"]);
    let mut route = route_result(
        crate::AskMode::direct_answer(),
        true,
        OutputResponseShape::Strict,
    );
    route.route_reason =
        "capability_ref=config.plan_change field_path=server.port value=8787".to_string();
    let loop_state = LoopState::new(2);
    let actions = vec![AgentAction::CallTool {
        tool: "config_edit".to_string(),
        args: json!({
            "action": "plan_config_change",
            "path": "configs/config.toml",
            "field_path": "server.port",
            "value": 8787
        }),
    }];

    let normalized = normalize_planned_actions_with_original(
        &state,
        Some(&route),
        &loop_state,
        "structured_config_contract",
        Some("structured_config_contract"),
        None,
        actions,
    );

    assert!(
        normalized.iter().any(action_targets_config_edit),
        "normalized actions: {normalized:?}"
    );
}

#[test]
fn rustclaw_config_guard_capability_ref_validation_rewrites_to_guard_config() {
    let mut route = base_route_result();
    route.route_reason = "capability_ref=config.guard_rustclaw_config".to_string();
    route.resolved_intent =
        "Validate the selected RustClaw config with semantic guard profile.".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();
    let actions = vec![AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args: json!({
            "action": "validate",
            "path": "configs/config.toml",
            "format": "toml",
            "validation_profile": "rustclaw_semantic_guard",
        }),
    }];

    let rewritten = rewrite_rustclaw_config_validation_to_guard(Some(&route), None, actions);

    let args = expect_planned_call(&rewritten[0], "config_basic", "guard_rustclaw_config");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some("configs/config.toml")
    );
    assert_eq!(args.get("format").and_then(Value::as_str), Some("toml"));
}

#[test]
fn rustclaw_config_syntax_only_validation_keeps_validate_action() {
    let mut route = base_route_result();
    route.resolved_intent = "Validate TOML syntax only.".to_string();
    route.output_contract.locator_hint = "configs/config.toml".to_string();
    let actions = vec![AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args: json!({
            "action": "validate",
            "path": "configs/config.toml",
            "format": "toml",
            "validation_profile": "syntax_only",
        }),
    }];

    let rewritten = rewrite_rustclaw_config_validation_to_guard(Some(&route), None, actions);

    expect_planned_call(&rewritten[0], "config_basic", "validate");
}

#[test]
fn command_output_summary_preserves_structured_config_validate() {
    let state = test_state();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::CommandOutputSummary;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();
    let actions = vec![
        AgentAction::CallTool {
            tool: "config_basic".to_string(),
            args: json!({
                "action": "validate",
                "path": "/home/guagua/rustclaw/configs/config.toml",
                "format": "toml",
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["step_1".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "Validate configs/config.toml as TOML and answer in one sentence.",
        Some("/home/guagua/rustclaw/configs/config.toml"),
        actions,
    );

    let args = expect_planned_call(&normalized[0], "config_basic", "validate");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some("configs/config.toml")
    );
    assert!(
        normalized.iter().all(
            |action| !planned_call_is(action, "config_edit", "guard_config")
                && !planned_call_is(action, "config_basic", "read_field")
        ),
        "normalized actions: {normalized:?}"
    );
}

#[test]
fn plain_main_config_validation_rewrites_to_planner_guard_when_contract_allows_guard() {
    let state = test_state();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ConfigRiskAssessment;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();
    let actions = vec![
        AgentAction::CallTool {
            tool: "config_basic".to_string(),
            args: json!({
                "action": "validate",
                "path": "/home/guagua/rustclaw/configs/config.toml",
                "format": "toml",
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
        "check main config for obvious configuration issues",
        Some("/home/guagua/rustclaw/configs/config.toml"),
        actions,
    );

    let args = expect_planned_call(&normalized[0], "config_basic", "guard_rustclaw_config");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some("configs/config.toml")
    );
}

#[test]
fn main_config_content_summary_read_rewrites_to_guard() {
    let state = test_state_with_registry();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": "/home/guagua/rustclaw/configs/config.toml",
                "mode": "head",
                "n": 120,
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["step_1".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "check main config for obvious configuration issues",
        Some("/home/guagua/rustclaw/configs/config.toml"),
        actions,
    );

    let args = expect_planned_call(&normalized[0], "config_basic", "guard_rustclaw_config");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some("configs/config.toml")
    );
}

#[test]
fn guard_config_with_invalid_product_locator_uses_main_config_default() {
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ConfigValidation;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/home/guagua/rustclaw/rustclaw".to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "config_basic".to_string(),
        args: json!({
            "action": "guard_rustclaw_config",
            "path": "/home/guagua/rustclaw/rustclaw",
        }),
    }];

    let normalized = repair_guard_config_default_path_for_invalid_locator(
        Some(&route),
        Some("/home/guagua/rustclaw/rustclaw"),
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
fn config_validation_contract_rewrites_broad_read_to_validate() {
    let mut route = base_route_result();
    route.output_contract.semantic_kind = OutputSemanticKind::ConfigValidation;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": "configs/config.toml",
                "mode": "head",
                "n": 500,
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["step_1".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let rewritten = rewrite_config_validation_read_plan_to_validate(Some(&route), None, actions);

    let args = expect_planned_call(&rewritten[0], "config_basic", "validate");
    assert_eq!(
        args.get("validation_profile").and_then(Value::as_str),
        Some("syntax_only")
    );
    assert_eq!(args.get("format").and_then(Value::as_str), Some("toml"));
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some("configs/config.toml")
    );
}

#[test]
fn config_validation_capability_ref_rewrites_broad_read_without_semantic_kind() {
    let mut route = base_route_result();
    route.route_reason = "capability_ref=config.validate".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": "configs/config.toml",
                "mode": "head",
                "n": 500,
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["step_1".to_string()],
        },
    ];

    let rewritten = rewrite_config_validation_read_plan_to_validate(Some(&route), None, actions);

    let args = expect_planned_call(&rewritten[0], "config_basic", "validate");
    assert_eq!(
        args.get("validation_profile").and_then(Value::as_str),
        Some("syntax_only")
    );
    assert_eq!(args.get("format").and_then(Value::as_str), Some("toml"));
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some("configs/config.toml")
    );
}

#[test]
fn config_validation_contract_normalizes_tool_read_to_validate() {
    let state = test_state();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ConfigValidation;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();
    route.resolved_intent =
        "Validate TOML syntax of configs/config.toml and answer pass or fail".to_string();
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": "/home/guagua/rustclaw/configs/config.toml",
                "mode": "head",
                "n": 120,
            }),
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
        &LoopState::new(1),
        "Validate only the TOML syntax of configs/config.toml and answer pass or fail.",
        Some("/home/guagua/rustclaw/configs/config.toml"),
        actions,
    );

    let args = expect_planned_call(&normalized[0], "config_basic", "validate");
    assert_eq!(
        args.get("validation_profile").and_then(Value::as_str),
        Some("syntax_only")
    );
    assert_eq!(args.get("format").and_then(Value::as_str), Some("toml"));
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some("configs/config.toml")
    );
    assert!(
        normalized
            .iter()
            .all(|action| !planned_call_is(action, "fs_basic", "read_text_range")),
        "normalized actions: {normalized:?}"
    );
}

#[test]
fn config_validation_contract_normalizes_legacy_system_validate_structured_to_validate() {
    let state = test_state();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ConfigValidation;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/configs/app_config.toml".to_string();
    route.resolved_intent =
        "Validate app_config.toml and briefly say whether it is readable".to_string();
    let actions = vec![
        AgentAction::CallTool {
            tool: "system_basic".to_string(),
            args: json!({
                "action": "validate_structured",
                "path": "scripts/nl_tests/fixtures/device_local/configs/app_config.toml",
                "format": "toml",
            }),
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = normalize_planned_actions(
            &state,
            Some(&route),
            &LoopState::new(1),
            "验证 scripts/nl_tests/fixtures/device_local/configs/app_config.toml 是否是可读配置，并简短说明结果。",
            None,
            actions,
        );

    let args = expect_planned_call(&normalized[0], "config_basic", "validate");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some("scripts/nl_tests/fixtures/device_local/configs/app_config.toml")
    );
    assert_eq!(args.get("format").and_then(Value::as_str), Some("toml"));
    assert!(
        normalized.iter().all(|action| !planned_call_is(
            action,
            "system_basic",
            "validate_structured"
        )),
        "normalized actions: {normalized:?}"
    );
}

#[test]
fn config_validation_contract_normalizes_config_field_read_to_validate() {
    let state = test_state();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ConfigValidation;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();
    route.resolved_intent =
        "Validate TOML syntax of configs/config.toml and answer pass or fail".to_string();
    let actions = vec![AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args: json!({
            "action": "read_field",
            "path": "/home/guagua/rustclaw/configs/config.toml",
            "field_path": "memory",
        }),
    }];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "Validate only the TOML syntax of configs/config.toml and answer pass or fail.",
        Some("/home/guagua/rustclaw/configs/config.toml"),
        actions,
    );

    let args = expect_planned_call(&normalized[0], "config_basic", "validate");
    assert_eq!(
        args.get("validation_profile").and_then(Value::as_str),
        Some("syntax_only")
    );
    assert_eq!(args.get("format").and_then(Value::as_str), Some("toml"));
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some("configs/config.toml")
    );
    assert!(
        normalized
            .iter()
            .all(|action| !planned_call_is(action, "config_basic", "read_field")),
        "normalized actions: {normalized:?}"
    );
}

#[test]
fn unrequested_path_like_config_field_read_rewrites_to_validate() {
    let root = TempDirGuard::new("unrequested_path_like_config_field");
    let config_dir = root.path.join("configs");
    fs::create_dir_all(&config_dir).expect("create config dir");
    let config_path = config_dir.join("app_config.toml");
    fs::write(&config_path, "[app]\nname = \"demo\"\n").expect("write config");
    let config_text = config_path.display().to_string();

    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = config_text.clone();
    route.resolved_intent = format!("Validate TOML syntax of {config_text}.");
    let actions = vec![AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args: json!({
            "action": "read_field",
            "path": config_text,
            "field_path": "no_such_note.md",
        }),
    }];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "Validate only whether configs/app_config.toml can be parsed as TOML.",
        Some(config_path.display().to_string().as_str()),
        actions,
    );

    let args = expect_planned_call(&normalized[0], "config_basic", "validate");
    assert_eq!(
        args.get("validation_profile").and_then(Value::as_str),
        Some("syntax_only")
    );
    assert_eq!(args.get("format").and_then(Value::as_str), Some("toml"));
    assert!(
        normalized
            .iter()
            .all(|action| !planned_call_is(action, "config_basic", "read_field")),
        "normalized actions: {normalized:?}"
    );
}

#[test]
fn explicit_path_like_config_field_read_is_preserved_when_user_mentions_field() {
    let root = TempDirGuard::new("explicit_path_like_config_field");
    let config_dir = root.path.join("configs");
    fs::create_dir_all(&config_dir).expect("create config dir");
    let config_path = config_dir.join("app_config.toml");
    fs::write(&config_path, "[app]\nname = \"demo\"\n").expect("write config");
    let config_text = config_path.display().to_string();

    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = config_text.clone();
    route.resolved_intent = format!("Read field no_such_note.md from {config_text}.");
    let actions = vec![AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args: json!({
            "action": "read_field",
            "path": config_text,
            "field_path": "no_such_note.md",
        }),
    }];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "Read field no_such_note.md from configs/app_config.toml.",
        Some(config_path.display().to_string().as_str()),
        actions,
    );

    let args = expect_planned_call(&normalized[0], "config_basic", "read_field");
    assert_eq!(
        args.get("field_path").and_then(Value::as_str),
        Some("no_such_note.md")
    );
}

#[test]
fn rustclaw_config_section_header_field_reads_rewrite_to_guard_config() {
    let state = test_state();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();
    let actions = vec![
        AgentAction::CallTool {
            tool: "config_basic".to_string(),
            args: json!({
                "action": "list_keys",
                "path": "/home/guagua/rustclaw/configs/config.toml",
            }),
        },
        AgentAction::CallTool {
            tool: "config_basic".to_string(),
            args: json!({
                "action": "read_fields",
                "path": "/home/guagua/rustclaw/configs/config.toml",
                "field_paths": ["[server]", "[security]", "[auth]"],
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["step_2".to_string()],
        },
    ];

    let normalized = normalize_planned_actions(
            &state,
            Some(&route),
            &LoopState::new(1),
            "Inspect RustClaw configuration file configs/config.toml for security or risk-related settings and present only the important findings.",
            Some("/home/guagua/rustclaw/configs/config.toml"),
            actions,
        );

    let args = expect_planned_call(&normalized[0], "config_basic", "guard_rustclaw_config");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some("configs/config.toml")
    );
    assert_eq!(args.get("format").and_then(Value::as_str), Some("toml"));
    assert!(
        normalized
            .iter()
            .all(|action| !planned_call_is(action, "config_basic", "read_fields")),
        "normalized actions: {normalized:?}"
    );
}

#[test]
fn config_risk_assessment_rewrites_key_listing_to_guard_config() {
    let state = test_state();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::ConfigRiskAssessment;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();
    let actions = vec![AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args: json!({
            "action": "list_keys",
            "path": "/home/guagua/rustclaw/configs/config.toml",
        }),
    }];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "Structured RustClaw config risk assessment.",
        Some("/home/guagua/rustclaw/configs/config.toml"),
        actions,
    );

    let args = expect_planned_call(&normalized[0], "config_basic", "guard_rustclaw_config");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some("configs/config.toml")
    );
    assert_eq!(args.get("format").and_then(Value::as_str), Some("toml"));
    assert!(
        normalized
            .iter()
            .all(|action| !planned_call_is(action, "config_basic", "list_keys")),
        "normalized actions: {normalized:?}"
    );
}

#[test]
fn config_risk_assessment_rewrites_file_head_read_to_guard_config() {
    let state = test_state();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::ConfigRiskAssessment;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "read_text_range",
            "path": "/home/guagua/rustclaw/configs/config.toml",
            "mode": "head",
            "n": 120,
        }),
    }];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "Structured RustClaw config risk assessment.",
        Some("/home/guagua/rustclaw/configs/config.toml"),
        actions,
    );

    let args = expect_planned_call(&normalized[0], "config_basic", "guard_rustclaw_config");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some("configs/config.toml")
    );
    assert!(
        normalized
            .iter()
            .all(|action| !planned_call_is(action, "fs_basic", "read_text_range")),
        "normalized actions: {normalized:?}"
    );
}

#[test]
fn config_risk_assessment_rewrites_config_edit_guard_to_preferred_config_basic_guard() {
    let state = test_state();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::ConfigRiskAssessment;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();
    let actions = vec![AgentAction::CallTool {
        tool: "config_edit".to_string(),
        args: json!({
            "action": "guard_config",
            "path": "configs/config.toml",
            "format": "toml",
        }),
    }];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "Structured RustClaw config risk assessment.",
        None,
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
fn rustclaw_main_config_content_excerpt_broad_read_rewrites_to_guard_config() {
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
            "mode": "head",
            "n": 120,
        }),
    }];

    let normalized = normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "Summarize the main config after observing current-task evidence.",
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

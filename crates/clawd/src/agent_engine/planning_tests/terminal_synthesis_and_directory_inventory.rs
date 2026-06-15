use super::*;

#[test]
fn quantity_compare_preserves_scalar_plus_text_evidence_for_explicit_files() {
    let root = TempDirGuard::new("quantity_scalar_plus_text");
    fs::write(
        root.path.join("Cargo.toml"),
        r#"[workspace]
members = []

[workspace.package]
version = "0.1.7"
"#,
    )
    .expect("write workspace cargo");
    fs::write(root.path.join("README.md"), "RustClaw v0.1.7\n").expect("write readme");

    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    let cargo_path = root.path.join("Cargo.toml");
    let readme_path = root.path.join("README.md");
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "extract_field",
                "path": cargo_path.display().to_string(),
                "field_path": "package.version"
            }),
        },
        AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: serde_json::json!({
                "path": readme_path.display().to_string()
            }),
        },
    ];
    let normalized = super::super::normalize_planned_actions(
            &state,
            Some(&route),
            &LoopState::new(2),
            "Read workspace package version from Cargo.toml and compare it with the version mentioned in README.md",
            Some(cargo_path.to_string_lossy().as_ref()),
            actions,
        );
    assert!(!normalized.iter().any(|action| matches!(
        action,
        AgentAction::CallSkill { skill, args }
            if skill == "system_basic"
                && args.get("action").and_then(Value::as_str) == Some("path_batch_facts")
    )));
    assert!(matches!(
        normalized.first(),
        Some(AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args })
            if skill == "config_basic"
                && args.get("action").and_then(Value::as_str) == Some("read_field")
                && args.get("path").and_then(Value::as_str)
                    == Some(cargo_path.to_string_lossy().as_ref())
                && args.get("field_path").and_then(Value::as_str)
                    == Some("workspace.package.version")
    ));
    assert!(matches!(
        normalized.get(1),
        Some(AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args })
            if skill == "fs_basic"
                && args.get("action").and_then(Value::as_str) == Some("read_text_range")
                && args.get("path").and_then(Value::as_str)
                    == Some(readme_path.to_string_lossy().as_ref())
    ));
    assert!(matches!(
        normalized.iter().find(|action| matches!(action, AgentAction::SynthesizeAnswer { .. })),
        Some(AgentAction::SynthesizeAnswer { evidence_refs })
            if evidence_refs == &vec!["step_1".to_string(), "step_2".to_string()]
    ));
    assert!(!should_force_actionable_plan_repair(
        &state,
        Some(&route),
        &LoopState::new(2),
        &normalized
    ));
}

#[test]
fn structured_scalar_compare_accepts_compare_paths_for_file_metadata() {
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: serde_json::json!({
            "action": "compare_paths",
            "left_path": "Cargo.lock",
            "right_path": "Cargo.toml"
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
fn observation_only_terminal_answer_appends_synthesis_for_builtin_observation() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: serde_json::json!({
            "action": "inventory_dir",
            "path": "logs",
            "files_only": true,
            "sort_by": "mtime_desc",
            "max_entries": 2
        }),
    }];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        false,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::None;

    let normalized = super::super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "列出 logs 最近修改的 2 个文件名，并判断更像运行日志还是测试残留",
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
fn observation_only_terminal_answer_keeps_config_basic_scalar_finalizer() {
    let actions = vec![AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args: serde_json::json!({
            "action": "read_field",
            "path": "configs/skills_registry.toml",
            "field_path": "run_cmd.planner_kind"
        }),
    }];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.locator_hint = "configs/skills_registry.toml".to_string();

    let normalized = super::super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "在 configs/skills_registry.toml 里找到 run_cmd 的 planner_kind",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 1);
    assert!(matches!(
        &normalized[0],
        AgentAction::CallTool { tool, args }
            if tool == "config_basic"
                && args.get("action").and_then(|value| value.as_str()) == Some("read_field")
    ));
}

#[test]
fn content_evidence_doc_parse_observation_appends_synthesis() {
    let actions = vec![AgentAction::CallSkill {
        skill: "doc_parse".to_string(),
        args: serde_json::json!({
            "action": "parse_doc",
            "path": "release_checklist.md",
            "max_chars": 12000
        }),
    }];
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "release_checklist.md".to_string();

    let normalized = super::super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "读一下 release_checklist.md，然后一句话告诉我最先该做什么",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 3);
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
fn terminal_synthesize_answer_appends_delivery_respond() {
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "path_batch_facts",
                "paths": ["missing.md"],
                "include_missing": true
            }),
        },
        AgentAction::CallSkill {
            skill: "doc_parse".to_string(),
            args: serde_json::json!({
                "action": "extract_key_points",
                "path": "README.md"
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["step_1".to_string(), "step_2".to_string()],
        },
    ];

    let normalized = super::super::append_respond_for_terminal_synthesize_answer(actions);

    assert_eq!(normalized.len(), 4);
    assert!(matches!(
        normalized.last(),
        Some(AgentAction::Respond { content }) if content == "{{last_output}}"
    ));
}

#[test]
fn observed_terminal_synthesis_replaces_concrete_respond_with_placeholder() {
    let mut loop_state = LoopState::new(3);
    loop_state.has_tool_or_skill_output = true;
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    let actions = vec![
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "first sentence. second sentence.".to_string(),
        },
    ];

    let rewritten =
        rewrite_observed_terminal_synthesis_concrete_respond(Some(&route), &loop_state, actions);

    assert_eq!(rewritten.len(), 2);
    assert!(matches!(
        &rewritten[0],
        AgentAction::SynthesizeAnswer { evidence_refs }
            if evidence_refs == &vec!["last_output".to_string()]
    ));
    assert!(matches!(
        &rewritten[1],
        AgentAction::Respond { content } if content == "{{last_output}}"
    ));
}

#[test]
fn observed_terminal_synthesis_keeps_service_status_concrete_respond() {
    let mut loop_state = LoopState::new(3);
    loop_state.has_tool_or_skill_output = true;
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    let actions = vec![
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "clawd running; clawd_log.keyword_error_count=43".to_string(),
        },
    ];

    let rewritten =
        rewrite_observed_terminal_synthesis_concrete_respond(Some(&route), &loop_state, actions);

    assert_eq!(rewritten.len(), 2);
    assert!(matches!(
        &rewritten[1],
        AgentAction::Respond { content }
            if content == "clawd running; clawd_log.keyword_error_count=43"
    ));
}

#[test]
fn observed_terminal_synthesis_keeps_structurally_grounded_concrete_respond() {
    use crate::executor::{StepExecutionResult, StepExecutionStatus};

    let mut loop_state = LoopState::new(3);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "tree_summary",
                "tree": {
                    "children": [
                        {
                            "kind": "file",
                            "path": "prompts/schemas/intent_normalizer.schema.json",
                            "size_bytes": 13160
                        }
                    ]
                }
            })
            .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    let answer =
        "intent_normalizer.schema.json 最大（13160 字节），描述用户意图解析输出。".to_string();
    let actions = vec![
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["round:2.step:3".to_string()],
        },
        AgentAction::Respond {
            content: answer.clone(),
        },
    ];

    let rewritten =
        rewrite_observed_terminal_synthesis_concrete_respond(Some(&route), &loop_state, actions);

    assert_eq!(rewritten.len(), 1);
    assert!(matches!(
        &rewritten[0],
        AgentAction::Respond { content } if content == &answer
    ));
}

#[test]
fn observed_terminal_synthesis_keeps_identifier_grounded_summary_respond() {
    use crate::executor::{StepExecutionResult, StepExecutionStatus};

    let mut loop_state = LoopState::new(3);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "read_range",
                "excerpt": "1|# Device Local Fixture\n2|\n3|This directory contains stable local files for RustClaw NL regression tests.",
                "path": "/tmp/README.md"
            })
            .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    let answer = "该目录为 RustClaw NL 回归测试提供稳定的本地文件样本。".to_string();
    let actions = vec![
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: answer.clone(),
        },
    ];

    let rewritten =
        rewrite_observed_terminal_synthesis_concrete_respond(Some(&route), &loop_state, actions);

    assert_eq!(rewritten.len(), 1);
    assert!(matches!(
        &rewritten[0],
        AgentAction::Respond { content } if content == &answer
    ));
}

#[test]
fn observed_terminal_synthesis_drops_redundant_synthesis_for_fs_basic_interface_summary() {
    use crate::executor::{StepExecutionResult, StepExecutionStatus};

    let mut loop_state = LoopState::new(3);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "read_range",
                "path": "/tmp/fs_basic.md",
                "resolved_path": "/tmp/fs_basic.md",
                "end_line": 92,
                "total_lines": 92,
                "line_safety": {
                    "truncated_lines": 0,
                    "compacted_lines": 0,
                    "raw": false,
                    "max_line_chars": 800
                },
                "excerpt": "1|## fs_basic - planner-facing filesystem tool\n2|Use call_tool fs_basic for filesystem tasks.\n3|runtime maps its actions to stable backing tools such as system_basic, fs_search, and file builtins.\n4|Actions: stat_paths, list_dir, count_entries, read_text_range, find_entries, grep_text, compare_paths, write_text, append_text, make_dir, remove_path."
            })
            .to_string(),
        ),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    let answer = "fs_basic is a virtual planner-facing filesystem tool that maps structured actions such as read_text_range and list_dir to stable backing tools like system_basic, fs_search, and file builtins.".to_string();
    let actions = vec![
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: answer.clone(),
        },
    ];

    let rewritten =
        rewrite_observed_terminal_synthesis_concrete_respond(Some(&route), &loop_state, actions);

    assert_eq!(rewritten.len(), 1);
    assert!(matches!(
        &rewritten[0],
        AgentAction::Respond { content } if content == &answer
    ));
}

#[test]
fn observation_only_terminal_answer_keeps_file_names_runtime_finalizer() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: serde_json::json!({
            "action": "inventory_dir",
            "path": "logs",
            "files_only": true,
            "sort_by": "mtime_desc",
            "max_entries": 2
        }),
    }];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        false,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::FileNames;

    let normalized = super::super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "只输出 logs 最近修改的 2 个文件名",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 1);
    assert!(planned_call_is(&normalized[0], "fs_basic", "list_dir"));
}

#[test]
fn general_directory_inventory_clears_file_only_filter() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: serde_json::json!({
            "action": "inventory_dir",
            "path": "/workspace/docs",
            "files_only": true,
            "names_only": true
        }),
    }];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::None;

    let normalized = super::super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "show the directory contents",
        None,
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "list_dir");
    assert_eq!(args.get("files_only").and_then(Value::as_bool), Some(false));
    assert_eq!(args.get("dirs_only").and_then(Value::as_bool), Some(false));
    assert_eq!(args.get("names_only").and_then(Value::as_bool), Some(false));
}

#[test]
fn directory_lookup_inventory_clears_file_only_even_with_file_names_semantic() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: serde_json::json!({
            "action": "inventory_dir",
            "path": "/workspace/docs",
            "files_only": true,
            "names_only": true
        }),
    }];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::FileNames;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::DirectoryLookup;

    let normalized = super::super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "inspect the directory contents",
        None,
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "list_dir");
    assert_eq!(args.get("files_only").and_then(Value::as_bool), Some(false));
    assert_eq!(args.get("dirs_only").and_then(Value::as_bool), Some(false));
    assert_eq!(args.get("names_only").and_then(Value::as_bool), Some(true));
}

#[test]
fn file_names_directory_inventory_preserves_file_only_filter() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: serde_json::json!({
            "action": "inventory_dir",
            "path": "/workspace/docs",
            "files_only": true,
            "names_only": true
        }),
    }];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::FileNames;

    let normalized = super::super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "output file names only",
        None,
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "list_dir");
    assert_eq!(args.get("files_only").and_then(Value::as_bool), Some(true));
    assert_eq!(args.get("names_only").and_then(Value::as_bool), Some(true));
}

#[test]
fn file_names_auto_locator_builds_list_dir_with_structural_extension_filter() {
    let root = TempDirGuard::new("file_names_auto_locator_ext");
    fs::write(root.path.join("alpha.md"), "alpha").expect("write md");
    fs::write(root.path.join("beta.txt"), "beta").expect("write txt");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::FileNames;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_path.clone();
    route.resolved_intent =
        "[CONTRACT_TEST_HINT]\nselector_extension=md\n[/CONTRACT_TEST_HINT]".to_string();
    let mut loop_state = LoopState::new(2);
    loop_state.round_no = 1;

    let plan = file_names_auto_locator_deterministic_plan_result(
        &test_state(),
        "return matching file names",
        Some(&route),
        &loop_state,
        &route.resolved_intent,
        Some(&route.resolved_intent),
        Some(root_path.as_str()),
    )
    .expect("file_names directory locator should build deterministic list_dir plan");

    assert_eq!(plan.plan_kind, PlanKind::Single);
    assert_eq!(plan.steps.len(), 1);
    let action = plan.steps[0].to_agent_action();
    let args = match &action {
        Some(AgentAction::CallTool { tool, args }) => {
            assert_eq!(tool, "fs_basic");
            args
        }
        other => panic!("expected fs_basic list_dir action, got {other:?}"),
    };
    assert_eq!(args.get("action").and_then(Value::as_str), Some("list_dir"));
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(root_path.as_str())
    );
    assert_eq!(args.get("files_only").and_then(Value::as_bool), Some(true));
    assert_eq!(args.get("names_only").and_then(Value::as_bool), Some(true));
    assert_eq!(
        args.get("ext_filter"),
        Some(&Value::Array(vec![Value::String("md".to_string())]))
    );
}

#[test]
fn file_names_auto_locator_does_not_inherit_extension_from_history_text() {
    let root = TempDirGuard::new("file_names_auto_locator_no_stale_ext");
    fs::write(root.path.join("clawd-dev.log"), "log").expect("write log");
    fs::write(root.path.join("act_plan.log"), "log").expect("write plan");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::FileNames;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_path.clone();
    route.resolved_intent = "list file names selector_limit=5".to_string();
    let user_text = "list logs file names selector_limit=5";
    let original_user_text = "previous ordered entries included hello.sh";
    let mut loop_state = LoopState::new(2);
    loop_state.round_no = 1;

    let plan = file_names_auto_locator_deterministic_plan_result(
        &test_state(),
        "return selected file names",
        Some(&route),
        &loop_state,
        user_text,
        Some(original_user_text),
        Some(root_path.as_str()),
    )
    .expect("file_names directory locator should build deterministic list_dir plan");

    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "fs_basic", "list_dir");
    assert!(args.get("ext_filter").is_none());
    assert_eq!(args.get("sort_by").and_then(Value::as_str), Some("name"));
}

#[test]
fn file_names_auto_locator_preserves_size_ranked_metadata() {
    let root = TempDirGuard::new("file_names_auto_locator_size_ranked");
    fs::write(root.path.join("large.log"), "large").expect("write large");
    fs::write(root.path.join("small.log"), "s").expect("write small");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::FileNames;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_path.clone();
    route.route_reason =
        "llm_semantic_contract_repair:file_names_contract_preserves_bounded_ordered_files_only_listing_with_size_format"
            .to_string();
    let user_text = "list top 3 files selector_limit=3 selector_sort_by=size_desc";
    let mut loop_state = LoopState::new(2);
    loop_state.round_no = 1;

    let plan = file_names_auto_locator_deterministic_plan_result(
        &test_state(),
        "return matching file names with sizes",
        Some(&route),
        &loop_state,
        user_text,
        Some(user_text),
        Some(root_path.as_str()),
    )
    .expect("file_names size-ranked directory locator should build deterministic list_dir plan");

    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "fs_basic", "list_dir");
    assert_eq!(args.get("files_only").and_then(Value::as_bool), Some(true));
    assert_eq!(args.get("names_only").and_then(Value::as_bool), Some(false));
    assert_eq!(args.get("max_entries").and_then(Value::as_u64), Some(3));
    assert_eq!(
        args.get("sort_by").and_then(Value::as_str),
        Some("size_desc")
    );
}

#[test]
fn directory_purpose_auto_locator_preserves_file_selector_for_selected_entry_judgment() {
    let root = TempDirGuard::new("directory_purpose_file_selector");
    fs::create_dir(root.path.join("generated")).expect("create child dir");
    for idx in 0..10 {
        fs::write(root.path.join(format!("note_{idx}.txt")), "fixture").expect("write note");
    }
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryPurposeSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_path.clone();
    route.output_contract.self_extension.list_selector = crate::OutputListSelector {
        target_kind: crate::OutputScalarCountTargetKind::File,
        target_kind_specified: true,
        limit: Some(5),
        sort_by: Some("name".to_string()),
        include_metadata: Some(false),
        include_hidden: None,
    };
    let mut loop_state = LoopState::new(2);
    loop_state.round_no = 1;

    let plan = directory_purpose_auto_locator_deterministic_plan_result(
        &test_state(),
        "judge selected entries from a directory listing",
        Some(&route),
        &loop_state,
        "selector_target_kind=file selector_limit=5",
        Some("selector_target_kind=file selector_limit=5"),
        Some(root_path.as_str()),
    )
    .expect("directory purpose selector should build deterministic listing plan");

    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "fs_basic", "list_dir");
    assert_eq!(args.get("files_only").and_then(Value::as_bool), Some(true));
    assert_eq!(args.get("dirs_only").and_then(Value::as_bool), Some(false));
    assert_eq!(args.get("names_only").and_then(Value::as_bool), Some(false));
    assert_eq!(args.get("max_entries").and_then(Value::as_u64), Some(5));
    assert_eq!(args.get("sort_by").and_then(Value::as_str), Some("name"));
}

#[test]
fn file_names_auto_locator_uses_structured_list_selector_without_reason_token() {
    let root = TempDirGuard::new("file_names_auto_locator_list_selector");
    fs::write(root.path.join("large.log"), "large").expect("write large");
    fs::write(root.path.join("small.log"), "s").expect("write small");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::FileNames;
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
    route.route_reason = "llm_semantic_contract_repair".to_string();
    let user_text = "return the selected files";
    let mut loop_state = LoopState::new(2);
    loop_state.round_no = 1;

    let plan = file_names_auto_locator_deterministic_plan_result(
        &test_state(),
        "return selected file names with sizes",
        Some(&route),
        &loop_state,
        user_text,
        Some(user_text),
        Some(root_path.as_str()),
    )
    .expect("file_names list-selector contract should build deterministic list_dir plan");

    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "fs_basic", "list_dir");
    assert_eq!(args.get("files_only").and_then(Value::as_bool), Some(true));
    assert_eq!(args.get("names_only").and_then(Value::as_bool), Some(false));
    assert_eq!(args.get("max_entries").and_then(Value::as_u64), Some(3));
    assert_eq!(
        args.get("sort_by").and_then(Value::as_str),
        Some("size_desc")
    );
}

#[test]
fn file_names_auto_locator_preserves_recent_modified_file_selector() {
    let root = TempDirGuard::new("file_names_auto_locator_recent_modified");
    fs::write(root.path.join("older.txt"), "old").expect("write older");
    fs::write(root.path.join("newer.txt"), "new").expect("write newer");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::FileNames;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_path.clone();
    route.output_contract.self_extension.list_selector = crate::OutputListSelector {
        target_kind: crate::OutputScalarCountTargetKind::File,
        target_kind_specified: true,
        limit: Some(2),
        sort_by: Some("mtime_desc".to_string()),
        include_metadata: Some(true),
        include_hidden: None,
    };
    route.resolved_intent =
        "return file names selector_limit=2 selector_sort_by=mtime_desc".to_string();
    let user_text = "return selected file names";
    let mut loop_state = LoopState::new(2);
    loop_state.round_no = 1;

    let plan = file_names_auto_locator_deterministic_plan_result(
        &test_state(),
        "return the two most recently modified file names",
        Some(&route),
        &loop_state,
        user_text,
        Some(user_text),
        Some(root_path.as_str()),
    )
    .expect("recent modified file_names selector should build deterministic list_dir plan");

    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "fs_basic", "list_dir");
    assert_eq!(args.get("files_only").and_then(Value::as_bool), Some(true));
    assert_eq!(args.get("names_only").and_then(Value::as_bool), Some(false));
    assert_eq!(args.get("max_entries").and_then(Value::as_u64), Some(2));
    assert_eq!(
        args.get("sort_by").and_then(Value::as_str),
        Some("mtime_desc")
    );
}

#[test]
fn file_names_auto_locator_preserves_structured_mtime_sort_selector_without_reason_token() {
    let root = TempDirGuard::new("file_names_auto_locator_stale_sort");
    fs::write(root.path.join("b.log"), "b").expect("write b");
    fs::write(root.path.join("a.log"), "a").expect("write a");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::FileNames;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_path.clone();
    route.output_contract.self_extension.list_selector = crate::OutputListSelector {
        target_kind: crate::OutputScalarCountTargetKind::File,
        target_kind_specified: true,
        limit: Some(5),
        sort_by: Some("mtime_asc".to_string()),
        include_metadata: Some(false),
        include_hidden: None,
    };
    route.resolved_intent = "return file names selector_limit=5".to_string();
    route.route_reason = "same operation different directory".to_string();
    let user_text = "return the first five file names";
    let mut loop_state = LoopState::new(2);
    loop_state.round_no = 1;

    let plan = file_names_auto_locator_deterministic_plan_result(
        &test_state(),
        "return selected file names",
        Some(&route),
        &loop_state,
        user_text,
        Some(user_text),
        Some(root_path.as_str()),
    )
    .expect("file_names directory locator should build deterministic list_dir plan");

    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "fs_basic", "list_dir");
    assert_eq!(
        args.get("sort_by").and_then(Value::as_str),
        Some("mtime_asc")
    );
    assert_eq!(args.get("names_only").and_then(Value::as_bool), Some(false));
}

#[test]
fn file_names_auto_locator_preserves_metadata_sort_with_machine_hint() {
    let root = TempDirGuard::new("file_names_auto_locator_supported_sort");
    fs::write(root.path.join("b.log"), "b").expect("write b");
    fs::write(root.path.join("a.log"), "a").expect("write a");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::FileNames;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_path.clone();
    route.output_contract.self_extension.list_selector = crate::OutputListSelector {
        target_kind: crate::OutputScalarCountTargetKind::File,
        target_kind_specified: true,
        limit: Some(5),
        sort_by: Some("mtime_asc".to_string()),
        include_metadata: Some(false),
        include_hidden: None,
    };
    route.resolved_intent =
        "return file names selector_limit=5 selector_sort_by=mtime_asc".to_string();
    let user_text = "return selected file names";
    let mut loop_state = LoopState::new(2);
    loop_state.round_no = 1;

    let plan = file_names_auto_locator_deterministic_plan_result(
        &test_state(),
        "return selected file names",
        Some(&route),
        &loop_state,
        user_text,
        Some(user_text),
        Some(root_path.as_str()),
    )
    .expect("file_names directory locator should build deterministic list_dir plan");

    let action = plan.steps[0].to_agent_action().expect("agent action");
    let args = expect_planned_call(&action, "fs_basic", "list_dir");
    assert_eq!(
        args.get("sort_by").and_then(Value::as_str),
        Some("mtime_asc")
    );
    assert_eq!(args.get("names_only").and_then(Value::as_bool), Some(false));
}

#[test]
fn file_names_contract_enforces_file_only_after_find_entries_inventory_rewrite() {
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "find_entries",
            "root": "/workspace/docs"
        }),
    }];
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::FileNames;

    let normalized = super::super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "output file names only",
        None,
        actions,
    );

    let Some((tool, args)) = planned_call(&normalized[0]) else {
        panic!("expected fs inventory call, got {:?}", normalized[0]);
    };
    assert_eq!(tool, "fs_basic");
    assert_eq!(
        args.get("action").and_then(Value::as_str),
        Some("find_entries")
    );
    assert_eq!(args.get("files_only").and_then(Value::as_bool), Some(true));
    assert_eq!(args.get("dirs_only").and_then(Value::as_bool), Some(false));
    assert_eq!(args.get("names_only").and_then(Value::as_bool), Some(true));
}

#[test]
fn strict_unclassified_directory_inventory_forces_metadata_for_fs_basic() {
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "list_dir",
            "path": "/workspace/logs",
            "names_only": true
        }),
    }];
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.semantic_kind = OutputSemanticKind::None;

    let normalized = super::super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "return the directory listing with requested details",
        None,
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "list_dir");
    assert_eq!(args.get("names_only").and_then(Value::as_bool), Some(false));
    assert_eq!(args.get("max_entries").and_then(Value::as_u64), Some(1000));
}

#[test]
fn strict_unclassified_system_inventory_forces_metadata_before_fs_rewrite() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: serde_json::json!({
            "action": "inventory_dir",
            "path": "/workspace/logs",
            "names_only": true
        }),
    }];
    let mut route = route_result(
        crate::AskMode::planner_execute_chat_wrapped(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.semantic_kind = OutputSemanticKind::None;

    let normalized = super::super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "return the directory listing with requested details",
        None,
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "list_dir");
    assert_eq!(args.get("names_only").and_then(Value::as_bool), Some(false));
    assert_eq!(args.get("max_entries").and_then(Value::as_u64), Some(1000));
}

#[test]
fn directory_names_contract_enforces_dirs_only_inventory() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: serde_json::json!({
            "action": "inventory_dir",
            "path": "/workspace",
            "files_only": true,
            "names_only": false
        }),
    }];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryNames;

    let normalized = super::super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "list top-level directory names only",
        None,
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "list_dir");
    assert_eq!(args.get("files_only").and_then(Value::as_bool), Some(false));
    assert_eq!(args.get("dirs_only").and_then(Value::as_bool), Some(true));
    assert_eq!(args.get("names_only").and_then(Value::as_bool), Some(true));
}

#[test]
fn directory_names_contract_does_not_invent_dirs_only_without_structured_filter() {
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "list_dir",
            "path": "/workspace/archive",
            "names_only": true
        }),
    }];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryNames;

    let normalized = super::super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "list entry names for the resolved directory",
        None,
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "list_dir");
    assert_eq!(args.get("files_only").and_then(Value::as_bool), Some(false));
    assert_eq!(args.get("dirs_only").and_then(Value::as_bool), Some(false));
    assert_eq!(args.get("names_only").and_then(Value::as_bool), Some(true));
}

#[test]
fn directory_names_contract_rewrites_filtered_list_dir_to_inventory() {
    let actions = vec![AgentAction::CallSkill {
        skill: "list_dir".to_string(),
        args: serde_json::json!({
            "path": "/workspace",
            "dirs_only": true
        }),
    }];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryNames;

    let normalized = super::super::normalize_planned_actions(
        &test_state_with_enabled_skills(&["list_dir", "system_basic"]),
        Some(&route),
        &LoopState::new(2),
        "list top-level directory names only",
        None,
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "list_dir");
    assert_eq!(args.get("files_only").and_then(Value::as_bool), Some(false));
    assert_eq!(args.get("dirs_only").and_then(Value::as_bool), Some(true));
    assert_eq!(args.get("names_only").and_then(Value::as_bool), Some(true));
    assert!(args.get("kind_filter").is_none());
}

#[test]
fn list_dir_kind_filter_file_rewrites_to_inventory_file_names() {
    let actions = vec![AgentAction::CallSkill {
        skill: "list_dir".to_string(),
        args: serde_json::json!({
            "path": "/workspace",
            "kind_filter": "file",
            "limit": 3
        }),
    }];

    let normalized = super::super::normalize_planned_actions(
        &test_state_with_enabled_skills(&["list_dir", "system_basic"]),
        None,
        &LoopState::new(2),
        "list file names",
        None,
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "list_dir");
    assert_eq!(args.get("files_only").and_then(Value::as_bool), Some(true));
    assert_eq!(args.get("dirs_only").and_then(Value::as_bool), Some(false));
    assert_eq!(args.get("names_only").and_then(Value::as_bool), Some(true));
    assert_eq!(args.get("limit").and_then(Value::as_u64), Some(3));
}

#[test]
fn file_paths_contract_rewrites_extension_inventory_to_fs_basic() {
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: serde_json::json!({
            "action": "inventory_dir",
            "path": ".",
            "files_only": true,
            "names_only": true,
            "ext_filter": ".toml",
            "max_entries": 5
        }),
    }];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::FilePaths;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;

    let normalized = super::super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "return five representative TOML file paths from the repository",
        None,
        actions,
    );

    match &normalized[0] {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            assert_eq!(skill, "fs_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("find_entries")
            );
            assert_eq!(args.get("root").and_then(Value::as_str), Some("."));
            assert_eq!(args.get("ext").and_then(Value::as_str), Some("toml"));
            assert_eq!(
                args.get("target_kind").and_then(Value::as_str),
                Some("file")
            );
            assert_eq!(args.get("max_results").and_then(Value::as_u64), Some(5));
            assert_eq!(args.get("recursive").and_then(Value::as_bool), Some(true));
        }
        other => panic!("expected fs_basic find_entries action, got {other:?}"),
    }
}

#[test]
fn file_paths_contract_rewrites_unfiltered_list_dir_with_extension_token_to_find_entries() {
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "list_dir",
            "path": "/home/guagua/rustclaw",
            "files_only": true,
            "names_only": true
        }),
    }];
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::FilePaths;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.resolved_intent =
        "Find all TOML files in the repository and mention representative ones".to_string();

    let normalized = super::super::normalize_planned_actions_with_original(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        &route.resolved_intent,
        Some("find toml files in this repo and briefly mention a few representative ones"),
        None,
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "find_entries");
    assert_eq!(
        args.get("root").and_then(Value::as_str),
        Some("/home/guagua/rustclaw")
    );
    assert_eq!(args.get("ext").and_then(Value::as_str), Some("toml"));
    assert_eq!(
        args.get("target_kind").and_then(Value::as_str),
        Some("file")
    );
    assert_eq!(args.get("recursive").and_then(Value::as_bool), Some(true));
}

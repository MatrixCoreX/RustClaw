use super::*;

#[test]
fn constructed_missing_stat_path_plan_rewrites_to_exact_find_entries() {
    let root = TempDirGuard::new("constructed_missing_stat_path");
    let locator = root.path.join("locator_smart");
    fs::create_dir_all(locator.join("case_only")).expect("create case dir");
    fs::write(locator.join("case_only/Report.MD"), "").expect("write report");

    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::respond_trace(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "stat_paths",
            "include_missing": true,
            "paths": ["locator_smart/Report.MD"]
        }),
    }];

    let normalized = super::super::normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "在 locator_smart 目录下查找 Report.MD 文件，仅输出路径",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 1);
    let args = expect_planned_call(&normalized[0], "fs_basic", "find_entries");
    assert_eq!(
        args.get("root").and_then(Value::as_str),
        Some("locator_smart")
    );
    assert_eq!(
        args.get("name_pattern").and_then(Value::as_str),
        Some("Report.MD")
    );
    assert_eq!(args.get("exact").and_then(Value::as_bool), Some(true));
}

#[test]
fn constructed_missing_stat_path_plan_rewrites_without_specific_semantic_kind() {
    let root = TempDirGuard::new("constructed_missing_stat_path_generic");
    let locator = root.path.join("locator_smart");
    fs::create_dir_all(locator.join("case_only")).expect("create case dir");
    fs::write(locator.join("case_only/Report.MD"), "").expect("write report");

    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let route = route_result(
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::Strict,
    );
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "stat_paths",
            "include_missing": true,
            "paths": ["locator_smart/Report.MD"]
        }),
    }];

    let normalized = super::super::normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "在目录 locator_smart 中查找文件 Report.MD 并输出其完整路径",
        None,
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "find_entries");
    assert_eq!(
        args.get("name_pattern").and_then(Value::as_str),
        Some("Report.MD")
    );
}

#[test]
fn file_paths_missing_stat_path_rewrites_to_selector_find_entries() {
    let root = TempDirGuard::new("file_paths_missing_stat_path_selector");
    let plan_dir = root.path.join("plan");
    fs::create_dir_all(&plan_dir).expect("create plan dir");
    fs::write(
        plan_dir.join("execution_intent_routing_repair_plan_20260509_done.md"),
        "",
    )
    .expect("write target md");
    fs::write(
        plan_dir.join("execution_retry_terminal_cases_20260510.md"),
        "",
    )
    .expect("write distractor md");
    fs::write(plan_dir.join("execution_intent_route_trace_cases.txt"), "")
        .expect("write txt distractor");

    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let missing = plan_dir.join("definitely_missing_20260511.md");
    let mut route = route_result(
        crate::AskMode::respond_trace(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FilePaths;
    route.output_contract.locator_hint = plan_dir.display().to_string();
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "stat_paths",
            "paths": [missing.display().to_string()]
        }),
    }];

    let normalized = super::super::normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "read plan/definitely_missing_20260511.md; if missing, search plan for execution_intent md files and only return found paths",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 1);
    let args = expect_planned_call(&normalized[0], "fs_basic", "find_entries");
    let expected_root = plan_dir.display().to_string();
    assert_eq!(
        args.get("root").and_then(Value::as_str),
        Some(expected_root.as_str())
    );
    assert_eq!(
        args.get("pattern").and_then(Value::as_str),
        Some("execution_intent")
    );
    assert_eq!(args.get("ext").and_then(Value::as_str), Some("md"));
    assert_eq!(
        args.get("target_kind").and_then(Value::as_str),
        Some("file")
    );
}

#[test]
fn constructed_directory_stat_path_plan_rewrites_to_find_entries_for_child_selector() {
    let root = TempDirGuard::new("constructed_directory_stat_path");
    let locator = root.path.join("locator_smart/fuzzy_top3");
    fs::create_dir_all(&locator).expect("create locator dir");
    fs::write(locator.join("abcd_report.md"), "").expect("write report");
    fs::write(locator.join("my_abcd.txt"), "").expect("write text");
    fs::write(locator.join("zz_abcd_backup.log"), "").expect("write log");

    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::act_with_chat_finalizer(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "stat_paths",
            "include_missing": true,
            "paths": ["locator_smart/fuzzy_top3"]
        }),
    }];

    let normalized = super::super::normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "去 locator_smart/fuzzy_top3 找 abcd",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 1);
    let args = expect_planned_call(&normalized[0], "fs_basic", "find_entries");
    assert_eq!(
        args.get("root").and_then(Value::as_str),
        Some("locator_smart/fuzzy_top3")
    );
    assert_eq!(
        args.get("name_pattern").and_then(Value::as_str),
        Some("abcd")
    );
    assert_eq!(args.get("exact").and_then(Value::as_bool), Some(false));
}

#[test]
fn constructed_directory_absolute_stat_path_rewrites_to_find_entries_for_child_selector() {
    let root = TempDirGuard::new("constructed_directory_absolute_stat_path");
    let locator = root.path.join("locator_smart/fuzzy_top3");
    fs::create_dir_all(&locator).expect("create locator dir");
    fs::write(locator.join("abcd_report.md"), "").expect("write report");
    fs::write(locator.join("my_abcd.txt"), "").expect("write text");
    fs::write(locator.join("zz_abcd_backup.log"), "").expect("write log");

    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::act_with_chat_finalizer(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
    let absolute_locator = locator.display().to_string();
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "stat_paths",
            "include_missing": true,
            "paths": [absolute_locator]
        }),
    }];

    let normalized = super::super::normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "去 locator_smart/fuzzy_top3 找 abcd",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 1);
    let args = expect_planned_call(&normalized[0], "fs_basic", "find_entries");
    assert_eq!(
        args.get("root").and_then(Value::as_str),
        Some(locator.display().to_string().as_str())
    );
    assert_eq!(
        args.get("name_pattern").and_then(Value::as_str),
        Some("abcd")
    );
    assert_eq!(args.get("exact").and_then(Value::as_bool), Some(false));
}

#[test]
fn split_dir_and_basename_stat_paths_rewrites_to_auto_locator_file() {
    let root = TempDirGuard::new("split_stat_auto_locator_file");
    let target_dir = root.path.join("locator_smart/case_only");
    fs::create_dir_all(&target_dir).expect("create case dir");
    let report = target_dir.join("Report.MD");
    fs::write(&report, "report").expect("write report");
    let target_dir_path = target_dir.display().to_string();
    let report_path = report.display().to_string();

    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::respond_trace(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = report_path.clone();
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "stat_paths",
            "fields": ["exists", "kind", "size", "modified"],
            "include_missing": true,
            "paths": [target_dir_path, "report.md"]
        }),
    }];

    let normalized = super::super::normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "Find the absolute path to report.md in scripts/nl_tests/fixtures/locator_smart/case_only",
        Some(&report_path),
        actions,
    );

    assert_eq!(normalized.len(), 1);
    let args = expect_planned_call(&normalized[0], "fs_basic", "stat_paths");
    assert_eq!(args.get("paths"), Some(&json!([report_path])));
    assert_eq!(
        args.get("fields"),
        Some(&json!(["exists", "kind", "size", "modified"]))
    );
}

#[test]
fn constructed_missing_stat_path_plan_preserves_explicit_full_path_check() {
    let root = TempDirGuard::new("explicit_missing_stat_path");
    fs::create_dir_all(root.path.join("locator_smart/case_only")).expect("create case dir");

    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::act_with_chat_finalizer(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "stat_paths",
            "include_missing": true,
            "paths": ["locator_smart/Report.MD"]
        }),
    }];

    let normalized = super::super::normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "检查 locator_smart/Report.MD 是否存在",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 1);
    let args = expect_planned_call(&normalized[0], "fs_basic", "stat_paths");
    assert_eq!(args.get("paths"), Some(&json!(["locator_smart/Report.MD"])));
}

#[test]
fn structured_scalar_compare_replaces_single_file_read_with_explicit_multi_file_path_facts() {
    let mut route = route_result(
        crate::AskMode::act_with_chat_finalizer(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    let actions = vec![AgentAction::CallSkill {
        skill: "doc_parse".to_string(),
        args: serde_json::json!({
            "action": "parse_doc",
            "path": "/home/guagua/rustclaw/README.md",
            "include_metadata": true
        }),
    }];
    let normalized = super::super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "compare README.md and AGENTS.md by size, then answer in one sentence",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 3);
    assert!(!normalized.iter().any(|action| matches!(
        action,
        AgentAction::CallSkill { skill, .. } if skill == "doc_parse"
    )));
    assert!(matches!(
        &normalized[0],
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "fs_basic"
                && args.get("action").and_then(Value::as_str) == Some("stat_paths")
                && args.get("paths").and_then(Value::as_array).is_some_and(|paths| {
                    paths.iter().any(|value| value.as_str() == Some("README.md"))
                        && paths.iter().any(|value| value.as_str() == Some("AGENTS.md"))
                })
    ));
    assert!(matches!(
        normalized.get(1),
        Some(AgentAction::SynthesizeAnswer { evidence_refs })
            if evidence_refs == &vec!["step_1".to_string()]
    ));
}

#[test]
fn structured_task_contract_targets_drive_multi_file_metadata_plan() {
    let mut route = route_result(
        crate::AskMode::act_with_chat_finalizer(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "README.md | AGENTS.md".to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "doc_parse".to_string(),
        args: serde_json::json!({
            "action": "parse_doc",
            "path": "README.md",
            "include_metadata": true
        }),
    }];

    let normalized = super::super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "compare these two targets by file metadata",
        None,
        actions,
    );

    assert!(matches!(
        &normalized[0],
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "fs_basic"
                && (
                    args.get("action").and_then(Value::as_str) == Some("stat_paths")
                        && args.get("paths").and_then(Value::as_array).is_some_and(|paths| {
                            paths.iter().any(|value| value.as_str() == Some("README.md"))
                                && paths.iter().any(|value| value.as_str() == Some("AGENTS.md"))
                        })
                    || args.get("action").and_then(Value::as_str) == Some("compare_paths")
                        && args.get("left_path").and_then(Value::as_str) == Some("README.md")
                        && args.get("right_path").and_then(Value::as_str) == Some("AGENTS.md")
                )
    ));
    assert!(matches!(
        normalized.get(1),
        Some(AgentAction::SynthesizeAnswer { evidence_refs })
            if evidence_refs == &vec!["step_1".to_string()]
    ));
}

#[test]
fn content_evidence_synthesize_only_plan_reads_structural_file_targets_first() {
    let temp = TempDirGuard::new("content_evidence_multi_read");
    let first = temp.path.join("first.md");
    let second = temp.path.join("second.md");
    fs::write(&first, "first file\nalpha\n").expect("write first file");
    fs::write(&second, "second file\nbeta\n").expect("write second file");

    let mut route = route_result(
        crate::AskMode::respond_trace(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.requires_content_evidence = true;
    let noisy_result_path = temp.path.join("mentioned_inside_result.toml");
    fs::write(&noisy_result_path, "ignored = true\n").expect("write noisy result file");
    let gate_context = serde_json::json!({
        "planner_loop": {
            "resolved_intent": "compare the file before last and last file",
        }
    });
    let plan_context = format!(
        "### RECENT_EXECUTION_EVENTS\n\
             - ts=2 kind=ask request=read {} result=mentions {}\n\
             - ts=1 kind=ask request=read {} result=ok\n\n{}",
        second.display(),
        noisy_result_path.display(),
        first.display(),
        gate_context
    );
    let actions = vec![
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = normalize_planned_actions_with_original_and_context(
        &test_state(),
        Some(&route),
        &LoopState::new(1),
        "compare the two previously referenced files in one sentence",
        None,
        Some(&plan_context),
        None,
        actions,
    );

    assert_eq!(normalized.len(), 4);
    let first_args = expect_planned_call(&normalized[0], "fs_basic", "read_text_range");
    let first_expected = first.display().to_string();
    assert_eq!(
        first_args.get("path").and_then(Value::as_str),
        Some(first_expected.as_str())
    );
    let second_args = expect_planned_call(&normalized[1], "fs_basic", "read_text_range");
    let second_expected = second.display().to_string();
    assert_eq!(
        second_args.get("path").and_then(Value::as_str),
        Some(second_expected.as_str())
    );
    assert!(matches!(
        normalized.get(2),
        Some(AgentAction::SynthesizeAnswer { evidence_refs })
            if evidence_refs == &vec!["step_1".to_string(), "step_2".to_string()]
    ));
    assert!(matches!(
        normalized.last(),
        Some(AgentAction::Respond { content }) if content == "{{last_output}}"
    ));
}

#[test]
fn content_evidence_partial_multi_file_read_appends_missing_structural_targets() {
    let temp = TempDirGuard::new("content_evidence_partial_multi_read");
    let first_rel = "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md";
    let second_rel = "scripts/nl_tests/fixtures/device_local/package.json";
    let first = temp.path.join(first_rel);
    let second = temp.path.join(second_rel);
    fs::create_dir_all(first.parent().expect("first parent")).expect("create first parent");
    fs::create_dir_all(second.parent().expect("second parent")).expect("create second parent");
    fs::write(&first, "- verify release\n- notify ops\n").expect("write first file");
    fs::write(&second, r#"{"name":"fixture-device"}"#).expect("write second file");

    let mut state = test_state();
    state.skill_rt.workspace_root = temp.path.clone();
    let mut route = route_result(
        crate::AskMode::respond_trace(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = first_rel.to_string();
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": "read_text_range",
                "path": first_rel,
                "mode": "head",
                "n": 60
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];
    let user_text = format!(
        "读一下 {first_rel} 开头，再看 {second_rel}，then 用一句中文说前者更像操作清单还是元数据文件"
    );

    let normalized = normalize_planned_actions_with_original_and_context(
        &state,
        Some(&route),
        &LoopState::new(1),
        &user_text,
        None,
        None,
        None,
        actions,
    );

    assert_eq!(normalized.len(), 4);
    let first_args = expect_planned_call(&normalized[0], "fs_basic", "read_text_range");
    assert_eq!(
        first_args.get("path").and_then(Value::as_str),
        Some(first_rel)
    );
    let second_args = expect_planned_call(&normalized[1], "fs_basic", "read_text_range");
    let second_expected = second.display().to_string();
    assert_eq!(
        second_args.get("path").and_then(Value::as_str),
        Some(second_expected.as_str())
    );
    assert!(matches!(
        normalized.get(2),
        Some(AgentAction::SynthesizeAnswer { evidence_refs })
            if evidence_refs == &vec!["step_1".to_string(), "step_2".to_string()]
    ));
    assert!(matches!(
        normalized.last(),
        Some(AgentAction::Respond { content }) if content == "{{last_output}}"
    ));
}

#[test]
fn content_evidence_current_auto_locator_does_not_append_previous_anchor_target() {
    let temp = TempDirGuard::new("content_evidence_current_auto_locator");
    let readme_rel = "scripts/nl_tests/fixtures/device_local/README.md";
    let service_rel = "scripts/nl_tests/fixtures/device_local/docs/service_notes.md";
    let readme = temp.path.join(readme_rel);
    let service = temp.path.join(service_rel);
    fs::create_dir_all(readme.parent().expect("readme parent")).expect("create readme parent");
    fs::create_dir_all(service.parent().expect("service parent")).expect("create service parent");
    fs::write(&readme, "RustClaw fixture\n").expect("write readme");
    fs::write(&service, "service notes\nline 2\nline 3\nline 4\n").expect("write service");

    let mut state = test_state();
    state.skill_rt.workspace_root = temp.path.clone();
    let mut route = route_result(
        crate::AskMode::respond_trace(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = service_rel.to_string();

    let readme_abs = readme.display().to_string();
    let service_abs = service.display().to_string();
    let plan_context = format!(
        "### ACTIVE_EXECUTION_ANCHOR\n\
         followup_bound_target: {readme_abs}\n\
         observed_bound_target: {readme_abs}\n\n\
         ### RECENT_EXECUTION_EVENTS\n\
         - ts=1 kind=ask request=read {readme_abs} result=ok\n\n\
         Resolved semantic request:\n\
         read {service_abs}"
    );
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": "read_text_range",
                "path": service_abs.clone(),
                "mode": "head",
                "n": 4
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["step_1".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];
    let original_user_text = format!("{service_rel} head 4");
    let execution_user_text = format!(
        "{original_user_text}\n\n\
         ### ACTIVE_EXECUTION_ANCHOR\n\
         followup_bound_target: {readme_abs}\n\
         observed_bound_target: {readme_abs}"
    );

    for auto_locator_path in [Some(service_abs.as_str()), None] {
        let normalized = normalize_planned_actions_with_original_and_context(
            &state,
            Some(&route),
            &LoopState::new(1),
            &execution_user_text,
            Some(&original_user_text),
            Some(&plan_context),
            auto_locator_path,
            actions.clone(),
        );

        let read_targets = normalized
            .iter()
            .filter_map(|action| {
                let (_tool, args) = planned_call(action)?;
                (args.get("action").and_then(Value::as_str) == Some("read_text_range"))
                    .then(|| args.get("path").and_then(Value::as_str).map(str::to_string))
                    .flatten()
            })
            .collect::<Vec<_>>();

        assert_eq!(read_targets, vec![service.display().to_string()]);
        assert!(!read_targets.iter().any(|path| path == &readme_abs));
        assert!(matches!(
            normalized.get(1),
            Some(AgentAction::SynthesizeAnswer { evidence_refs })
                if evidence_refs == &vec!["step_1".to_string()]
        ));
    }
}

#[test]
fn existence_multi_file_stat_paths_are_repaired_from_structural_targets() {
    let mut route = route_result(
        crate::AskMode::respond_trace(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.requires_content_evidence = true;
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "stat_paths",
            "paths": ["README.md", "-CN.md", "Cargo.toml", "no_such_file_20260513.txt"],
            "include_missing": true
        }),
    }];

    let normalized = super::super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "检查 README.md, README.zh-CN.md, Cargo.toml, and no_such_file_20260513.txt 是否存在，并用表格返回结果。",
        None,
        actions,
    );

    assert_eq!(normalized.len(), 1);
    assert!(matches!(
        &normalized[0],
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "fs_basic"
                && args.get("action").and_then(Value::as_str) == Some("stat_paths")
                && args.get("paths").and_then(Value::as_array).is_some_and(|paths| {
                    paths.iter().any(|value| value.as_str() == Some("README.md"))
                        && paths.iter().any(|value| value.as_str() == Some("README.zh-CN.md"))
                        && paths.iter().any(|value| value.as_str() == Some("Cargo.toml"))
                        && paths.iter().any(|value| value.as_str() == Some("no_such_file_20260513.txt"))
                        && !paths.iter().any(|value| value.as_str() == Some("-CN.md"))
                })
    ));
}

#[test]
fn explicit_multi_file_metadata_plan_is_not_duplicated_when_targets_are_covered() {
    let mut route = route_result(
        crate::AskMode::act_with_chat_finalizer(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: serde_json::json!({
            "action": "path_batch_facts",
            "paths": ["README.md", "AGENTS.md"]
        }),
    }];
    let normalized = super::super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "比较 README.md 和 AGENTS.md 的大小，并用一句话解释",
        None,
        actions,
    );

    assert_eq!(
        normalized
            .iter()
            .filter(|action| planned_call_is(action, "fs_basic", "stat_paths"))
            .count(),
        1
    );
}

#[test]
fn normalization_order_schema_aliases_before_multi_target_coverage() {
    let mut route = route_result(
        crate::AskMode::act_with_chat_finalizer(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "README.md | AGENTS.md".to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: serde_json::json!({
            "action": "path_batch_facts",
            "path_list": ["README.md", "AGENTS.md"]
        }),
    }];

    let normalized = super::super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "compare the two task-contract targets by file metadata",
        None,
        actions,
    );

    let path_fact_actions = normalized
        .iter()
        .filter_map(|action| match action {
            AgentAction::CallSkill { skill, args } if skill == "fs_basic" => Some(args),
            AgentAction::CallTool { tool, args } if tool == "fs_basic" => Some(args),
            _ => None,
        })
        .filter(|args| args.get("action").and_then(Value::as_str) == Some("stat_paths"))
        .collect::<Vec<_>>();
    assert_eq!(path_fact_actions.len(), 1);
    let args = path_fact_actions[0];
    assert!(args.get("path_list").is_none());
    assert!(args
        .get("paths")
        .and_then(Value::as_array)
        .is_some_and(|paths| {
            paths
                .iter()
                .any(|value| value.as_str() == Some("README.md"))
                && paths
                    .iter()
                    .any(|value| value.as_str() == Some("AGENTS.md"))
        }));
}

#[test]
fn multi_file_modified_time_compare_uses_metadata_not_whole_file_reads() {
    let mut route = route_result(
        crate::AskMode::act_with_chat_finalizer(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "README.md | AGENTS.md".to_string();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: serde_json::json!({"path": "README.md"}),
        },
        AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: serde_json::json!({"path": "AGENTS.md"}),
        },
    ];

    let normalized = super::super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "比较这两个文件哪个修改时间更新",
        None,
        actions,
    );

    assert!(!normalized.iter().any(|action| matches!(
        action,
        AgentAction::CallSkill { skill, .. } if skill == "read_file"
    )));
    assert!(matches!(
        &normalized[0],
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "fs_basic"
                && args.get("action").and_then(Value::as_str) == Some("stat_paths")
                && args.get("fields").and_then(Value::as_array).is_some_and(|fields| {
                    fields.iter().any(|value| value.as_str() == Some("modified"))
                })
    ));
}

#[test]
fn recent_scalar_equality_preserves_content_extract_plan_for_explicit_files() {
    let mut route = route_result(
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RecentScalarEqualityCheck;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "extract_field",
                "path": "Cargo.toml",
                "field_path": "package.version"
            }),
        },
        AgentAction::CallSkill {
            skill: "fs_search".to_string(),
            args: serde_json::json!({
                "action": "grep_text",
                "root": "README.md",
                "query": "version",
                "max_matches": 5
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["s0".to_string(), "s1".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];
    let normalized = super::super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        "Read workspace package version from Cargo.toml and compare it with the version mentioned in README.md",
        None,
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
                && args.get("field_path").and_then(Value::as_str) == Some("package.version")
    ));
    assert!(matches!(
        normalized.get(1),
        Some(AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args })
            if skill == "fs_basic"
                && args.get("action").and_then(Value::as_str) == Some("grep_text")
    ));
    assert!(!should_force_actionable_plan_repair(
        &test_state(),
        Some(&route),
        &LoopState::new(2),
        &normalized
    ));
}

#[test]
fn recent_scalar_equality_pair_paths_skip_content_read_deterministic_fallback() {
    let state = test_state();
    let mut route = route_result(
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RecentScalarEqualityCheck;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "Cargo.toml | README.md".to_string();
    let loop_state = LoopState::new(1);

    let normalized = super::super::normalize_planned_actions(
        &state,
        Some(&route),
        &loop_state,
        "compare two path facts",
        None,
        vec![],
    );

    assert!(
        normalized.is_empty(),
        "runtime must not inject content-read fallback before the planner: {normalized:?}"
    );
}

#[test]
fn recent_scalar_equality_pair_paths_uses_compare_paths_plan() {
    let mut route = route_result(
        crate::AskMode::respond_trace(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RecentScalarEqualityCheck;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "Cargo.toml | README.md".to_string();

    let normalized = super::super::normalize_planned_actions(
        &test_state(),
        Some(&route),
        &LoopState::new(1),
        "compare two path metadata targets",
        None,
        vec![AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "compare_paths",
                "left_path": "Cargo.toml",
                "right_path": "README.md"
            }),
        }],
    );

    assert_eq!(normalized.len(), 1);
    let args = expect_planned_call(&normalized[0], "fs_basic", "compare_paths");
    assert_eq!(
        args.get("left_path").and_then(Value::as_str),
        Some("Cargo.toml")
    );
    assert_eq!(
        args.get("right_path").and_then(Value::as_str),
        Some("README.md")
    );
}

#[test]
fn recent_scalar_file_pair_plan_reads_structured_field_and_text_evidence() {
    let root = TempDirGuard::new("recent_scalar_file_pair");
    fs::write(
        root.path.join("Cargo.toml"),
        r#"[workspace]
members = []

[workspace.package]
version = "0.1.7"

[workspace.dependencies]
toml = "0.8"
reqwest = { version = "0.12" }
"#,
    )
    .expect("write cargo manifest");
    fs::write(
        root.path.join("README.md"),
        "RustClaw release notes\nversion: 0.1.7\n",
    )
    .expect("write readme");

    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let cargo_path = root.path.join("Cargo.toml").display().to_string();
    let readme_path = root.path.join("README.md").display().to_string();
    let mut route = route_result(
        crate::AskMode::respond_trace(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RecentScalarEqualityCheck;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = format!("{cargo_path} | {readme_path}");
    route.resolved_intent =
        "Read workspace package version from Cargo.toml and compare it with README.md.".to_string();
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let normalized = super::super::normalize_planned_actions(
        &state,
        Some(&route),
        &loop_state,
        "Read workspace package version from Cargo.toml and compare it with README.md.",
        None,
        vec![
            AgentAction::CallTool {
                tool: "config_basic".to_string(),
                args: json!({
                    "action": "read_field",
                    "path": cargo_path,
                    "field_path": "workspace.package.version"
                }),
            },
            AgentAction::CallTool {
                tool: "fs_basic".to_string(),
                args: json!({
                    "action": "grep_text",
                    "path": readme_path,
                    "query": "version"
                }),
            },
        ],
    );

    let args = expect_planned_call(&normalized[0], "config_basic", "read_field");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(cargo_path.as_str())
    );
    assert_eq!(
        args.get("field_path").and_then(Value::as_str),
        Some("workspace.package.version")
    );

    let args = expect_planned_call(&normalized[1], "fs_basic", "grep_text");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(readme_path.as_str())
    );
    assert_eq!(args.get("query").and_then(Value::as_str), Some("version"));
    assert!(!should_force_actionable_plan_repair(
        &state,
        Some(&route),
        &loop_state,
        &normalized
    ));
}

#[test]
fn recent_scalar_contract_overrides_literal_command_guard_for_deterministic_plan() {
    let mut route = route_result(
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RecentScalarEqualityCheck;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;

    assert!(super::super::route_contract_defers_literal_command_to_planner(Some(&route)));
}

#[test]
fn recent_scalar_file_pair_plan_accepts_relative_route_locators() {
    let root = TempDirGuard::new("recent_scalar_relative_pair");
    fs::write(
        root.path.join("Cargo.toml"),
        r#"[workspace]
members = []

[workspace.package]
version = "0.1.7"
"#,
    )
    .expect("write cargo manifest");
    fs::write(root.path.join("README.md"), "version: 0.1.7\n").expect("write readme");

    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::respond_trace(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RecentScalarEqualityCheck;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "Cargo.toml|README.md".to_string();
    route.resolved_intent =
        "Extract workspace package version from Cargo.toml and the version mentioned in README.md, compare them, and answer whether they match in one sentence."
            .to_string();
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let normalized = super::super::normalize_planned_actions(
        &state,
        Some(&route),
        &loop_state,
        &route.resolved_intent,
        None,
        vec![
            AgentAction::CallTool {
                tool: "config_basic".to_string(),
                args: json!({
                    "action": "read_field",
                    "path": "Cargo.toml",
                    "field_path": "workspace.package.version"
                }),
            },
            AgentAction::CallTool {
                tool: "fs_basic".to_string(),
                args: json!({
                    "action": "grep_text",
                    "path": "README.md",
                    "query": "version"
                }),
            },
        ],
    );

    assert!(
        normalized.iter().any(|action| matches!(
            action,
            AgentAction::CallTool { tool, args } | AgentAction::CallSkill { skill: tool, args }
                if tool == "config_basic"
                    && args.get("action").and_then(Value::as_str) == Some("read_field")
                    && args.get("path").and_then(Value::as_str) == Some("Cargo.toml")
        )),
        "planner-supplied relative config read should be preserved: {normalized:?}"
    );
}

#[test]
fn recent_scalar_file_pair_plan_reads_two_structured_field_values() {
    let root = TempDirGuard::new("recent_scalar_two_structured_files");
    fs::create_dir_all(root.path.join("UI")).expect("ui dir");
    fs::create_dir_all(root.path.join("crates/clawd")).expect("clawd dir");
    fs::write(
        root.path.join("UI/package.json"),
        r#"{"name":"react-example","version":"0.0.0"}"#,
    )
    .expect("write package json");
    fs::write(
        root.path.join("crates/clawd/Cargo.toml"),
        r#"[package]
name = "clawd"
version = "0.1.0"
"#,
    )
    .expect("write cargo manifest");

    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let package_path = root.path.join("UI/package.json").display().to_string();
    let cargo_path = root
        .path
        .join("crates/clawd/Cargo.toml")
        .display()
        .to_string();
    let mut route = route_result(
        crate::AskMode::respond_trace(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RecentScalarEqualityCheck;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = package_path.clone();
    route.resolved_intent = "Compare UI/package.json name with crates/clawd/Cargo.toml package.name and output one scalar verdict.".to_string();
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;
    let prompt = "读取 UI/package.json 里的 name，再读取 crates/clawd/Cargo.toml 里的 package.name，最后只用一行输出：前者、后者、一样或不一样";

    let normalized = super::super::normalize_planned_actions(
        &state,
        Some(&route),
        &loop_state,
        prompt,
        None,
        vec![
            AgentAction::CallTool {
                tool: "config_basic".to_string(),
                args: json!({
                    "action": "read_field",
                    "path": package_path,
                    "field_path": "name"
                }),
            },
            AgentAction::CallTool {
                tool: "config_basic".to_string(),
                args: json!({
                    "action": "read_field",
                    "path": cargo_path,
                    "field_path": "package.name"
                }),
            },
        ],
    );
    assert_eq!(normalized.len(), 2);

    let first_args = expect_planned_call(&normalized[0], "config_basic", "read_field");
    assert_eq!(
        first_args.get("path").and_then(Value::as_str),
        Some(package_path.as_str())
    );
    assert_eq!(
        first_args.get("field_path").and_then(Value::as_str),
        Some("name")
    );

    let second_args = expect_planned_call(&normalized[1], "config_basic", "read_field");
    assert_eq!(
        second_args.get("path").and_then(Value::as_str),
        Some(cargo_path.as_str())
    );
    assert_eq!(
        second_args.get("field_path").and_then(Value::as_str),
        Some("package.name")
    );

    assert!(!should_force_actionable_plan_repair(
        &state,
        Some(&route),
        &loop_state,
        &normalized
    ));
}

#[tokio::test]
async fn plan_round_recent_scalar_file_pair_reaches_planner_without_pre_llm_shortcut() {
    let root = TempDirGuard::new("recent_scalar_plan_round_relative_pair");
    fs::write(
        root.path.join("Cargo.toml"),
        r#"[workspace]
members = []

[workspace.package]
version = "0.1.7"

[workspace.dependencies]
toml = "0.8"
reqwest = { version = "0.12" }
"#,
    )
    .expect("write cargo manifest");
    fs::write(root.path.join("README.md"), "version: 0.1.7\n").expect("write readme");

    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let prompt = "Read workspace package version from Cargo.toml and compare it with the version mentioned in README.md, then answer in one sentence.";
    let task = ClaimedTask {
        task_id: "recent-scalar-plan-round".to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: json!({ "text": prompt }).to_string(),
    };
    let mut route = route_result(
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RecentScalarEqualityCheck;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "Cargo.toml|README.md".to_string();
    route.resolved_intent =
        "Extract workspace package version from Cargo.toml and the version mentioned in README.md, compare them, and answer whether they match in one sentence."
            .to_string();
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;
    let policy = super::super::super::support::load_agent_loop_guard_policy(&state);

    let err = super::super::plan_round_actions(
        &state,
        &task,
        &route.resolved_intent,
        &route.resolved_intent,
        &policy,
        &loop_state,
        None,
        None,
        Some(&route),
        None,
    )
    .await
    .expect_err("recent scalar pair should reach planner instead of pre-LLM deterministic plan");
    assert!(
        err.contains("required prompt missing"),
        "expected missing planner prompt after deterministic shortcut removal, got: {err}"
    );
    assert!(
        !err.contains("plan_deterministic_recent_scalar_file_pair"),
        "old recent scalar deterministic fallback leaked into planner error: {err}"
    );
}

#[tokio::test]
async fn plan_round_recent_scalar_file_pair_single_auto_locator_uses_planner_path() {
    let root = TempDirGuard::new("recent_scalar_plan_round_single_locator");
    let cargo = root.path.join("Cargo.toml");
    fs::write(
        &cargo,
        r#"[workspace]
members = []

[workspace.package]
version = "0.1.7"

[workspace.dependencies]
toml = "0.8"
reqwest = { version = "0.12" }
"#,
    )
    .expect("write cargo manifest");
    fs::write(root.path.join("README.md"), "version: 0.1.7\n").expect("write readme");

    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let prompt = "Read workspace package version from Cargo.toml and compare it with the version mentioned in README.md, then answer in one sentence.";
    let task = ClaimedTask {
        task_id: "recent-scalar-plan-round-single-locator".to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: json!({ "text": prompt }).to_string(),
    };
    let mut route = route_result(
        crate::AskMode::act_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RecentScalarEqualityCheck;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = cargo.display().to_string();
    route.resolved_intent =
        "Compare workspace package version from Cargo.toml with version mentioned in README.md, answer in one sentence"
            .to_string();
    let cargo_auto = cargo.display().to_string();
    let planner_user_text = format!(
        "{}\n\n[AUTO_LOCATOR]\nResolved concrete path from default locator directory: {}\nUse this path as the target unless user explicitly overrides it.\n",
        route.resolved_intent, cargo_auto
    );
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;
    let policy = super::super::super::support::load_agent_loop_guard_policy(&state);

    let err = super::super::plan_round_actions(
        &state,
        &task,
        &route.resolved_intent,
        &planner_user_text,
        &policy,
        &loop_state,
        None,
        None,
        Some(&route),
        Some(cargo_auto.as_str()),
    )
    .await
    .expect_err("single auto locator route should reach planner instead of deterministic fallback");

    assert!(
        err.contains("required prompt missing"),
        "expected missing planner prompt after deterministic shortcut removal, got: {err}"
    );
    assert!(
        !err.contains("plan_deterministic_recent_scalar_file_pair"),
        "old recent scalar deterministic fallback leaked into planner error: {err}"
    );
}

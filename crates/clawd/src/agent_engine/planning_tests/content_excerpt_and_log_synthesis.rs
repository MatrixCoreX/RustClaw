use super::*;

#[test]
fn content_excerpt_summary_auto_locator_preserves_tail_slice_selector() {
    let root = TempDirGuard::new("content_excerpt_tail_slice_selector");
    let config_dir = root.path.join("configs");
    fs::create_dir_all(&config_dir).expect("create configs dir");
    let config = config_dir.join("config.toml");
    fs::write(
        &config,
        "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n",
    )
    .expect("write config");
    let config_path = config.display().to_string();
    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.delivery_required = false;
    route.output_contract.requires_content_evidence = true;
    route.resolved_intent = "Summarize bounded file excerpt. slice_mode=tail slice_n=5".to_string();
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let plan = content_excerpt_summary_auto_locator_deterministic_plan_result(
        &state,
        "read the requested bounded file excerpt and summarize it",
        Some(&route),
        &loop_state,
        Some(&config_path),
    )
    .expect("content excerpt summary should preserve route slice selector");

    let read_args = plan
        .steps
        .iter()
        .filter_map(|step| step.to_agent_action())
        .find_map(|action| match action {
            AgentAction::CallTool { tool, args }
                if tool == "fs_basic"
                    && args.get("action").and_then(Value::as_str) == Some("read_text_range") =>
            {
                Some(args)
            }
            _ => None,
        })
        .expect("expected fs_basic read_text_range evidence");
    assert_eq!(
        read_args.get("path").and_then(Value::as_str),
        Some(config_path.as_str())
    );
    assert_eq!(read_args.get("mode").and_then(Value::as_str), Some("tail"));
    assert_eq!(read_args.get("n").and_then(Value::as_u64), Some(5));
}

#[test]
fn content_excerpt_summary_auto_locator_abstains_without_slice_selector_for_repo_text() {
    let root = TempDirGuard::new("content_excerpt_no_slice_selector");
    let config_dir = root.path.join("configs");
    fs::create_dir_all(&config_dir).expect("create configs dir");
    let config = config_dir.join("config.toml");
    fs::write(&config, "one\ntwo\nthree\n").expect("write config");
    let config_path = config.display().to_string();
    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.delivery_required = false;
    route.output_contract.requires_content_evidence = true;
    route.resolved_intent =
        "Summarize a bounded file excerpt without machine slice metadata.".to_string();
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let plan = content_excerpt_summary_auto_locator_deterministic_plan_result(
        &state,
        "read an excerpt and summarize it",
        Some(&route),
        &loop_state,
        Some(&config_path),
    );

    assert!(
        plan.is_none(),
        "deterministic content-excerpt plan should not guess a default head range when slice metadata is absent"
    );
}

#[test]
fn content_excerpt_summary_directory_log_slice_uses_exact_log_file_read() {
    let root = TempDirGuard::new("content_excerpt_directory_log_slice");
    let logs_dir = root.path.join("logs");
    fs::create_dir_all(&logs_dir).expect("mkdir logs");
    let log = logs_dir.join("clawd.run.log");
    fs::write(
        &log,
        "INFO boot start\nINFO worker ready\nERROR provider retry\nINFO task succeeded\n",
    )
    .expect("write clawd log");
    fs::write(
        logs_dir.join("model_io.log"),
        "ERROR unrelated provider log\n",
    )
    .expect("write other log");
    let logs_path = logs_dir.display().to_string();
    let log_path = log.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = logs_path.clone();
    route.output_contract.delivery_required = false;
    route.output_contract.requires_content_evidence = true;
    route.resolved_intent =
        "List clawd related log files, read clawd.run.log tail. slice_mode=tail slice_n=20"
            .to_string();

    let plan = content_excerpt_summary_directory_log_slice_deterministic_plan_result(
        "read the selected log tail and synthesize a one-sentence status judgment",
        Some(&route),
        &LoopState::new(1),
        Some(&logs_path),
    )
    .expect("directory log slice should use exact file read plan");

    assert_eq!(plan.steps.len(), 4);
    let find_action = plan.steps[0].to_agent_action().unwrap();
    let find_args = expect_planned_call(&find_action, "fs_basic", "find_entries");
    assert_eq!(
        find_args.get("root").and_then(Value::as_str),
        Some(logs_path.as_str())
    );
    assert_eq!(
        find_args.get("pattern").and_then(Value::as_str),
        Some("clawd")
    );
    assert_eq!(
        find_args.get("target_kind").and_then(Value::as_str),
        Some("file")
    );
    let read_action = plan.steps[1].to_agent_action().unwrap();
    let read_args = expect_planned_call(&read_action, "fs_basic", "read_text_range");
    assert_eq!(
        read_args.get("path").and_then(Value::as_str),
        Some(log_path.as_str())
    );
    assert_eq!(read_args.get("mode").and_then(Value::as_str), Some("tail"));
    assert_eq!(read_args.get("n").and_then(Value::as_u64), Some(20));
    assert_eq!(plan.steps[2].action_type, "synthesize_answer");
    assert_eq!(plan.steps[3].action_type, "respond");
}

#[test]
fn content_excerpt_with_summary_directory_log_slice_uses_exact_log_file_read() {
    let root = TempDirGuard::new("content_excerpt_with_summary_directory_log_slice");
    let logs_dir = root.path.join("logs");
    fs::create_dir_all(&logs_dir).expect("mkdir logs");
    let log = logs_dir.join("clawd.run.log");
    fs::write(&log, "INFO boot start\nINFO worker ready\n").expect("write clawd log");
    fs::write(logs_dir.join("model_io.log"), "ERROR unrelated\n").expect("write other log");
    let logs_path = logs_dir.display().to_string();
    let log_path = log.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptWithSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = logs_path.clone();
    route.output_contract.delivery_required = false;
    route.output_contract.requires_content_evidence = true;
    route.resolved_intent = "target_path=clawd.run.log slice_mode=tail slice_n=20".to_string();

    let plan = content_excerpt_summary_directory_log_slice_deterministic_plan_result(
        "read the selected log tail and synthesize status",
        Some(&route),
        &LoopState::new(1),
        Some(&logs_path),
    )
    .expect("content-excerpt-with-summary should share exact directory log slice plan");

    assert_eq!(plan.steps.len(), 4);
    let find_action = plan.steps[0].to_agent_action().unwrap();
    let find_args = expect_planned_call(&find_action, "fs_basic", "find_entries");
    assert_eq!(
        find_args.get("root").and_then(Value::as_str),
        Some(logs_path.as_str())
    );
    assert_eq!(
        find_args.get("pattern").and_then(Value::as_str),
        Some("clawd")
    );
    let read_action = plan.steps[1].to_agent_action().unwrap();
    let read_args = expect_planned_call(&read_action, "fs_basic", "read_text_range");
    assert_eq!(
        read_args.get("path").and_then(Value::as_str),
        Some(log_path.as_str())
    );
    assert_eq!(read_args.get("mode").and_then(Value::as_str), Some("tail"));
    assert_eq!(read_args.get("n").and_then(Value::as_u64), Some(20));
    assert_eq!(plan.steps[2].action_type, "synthesize_answer");
    assert_eq!(plan.steps[3].action_type, "respond");
}

#[test]
fn content_excerpt_summary_directory_log_slice_accepts_comma_machine_tokens() {
    let root = TempDirGuard::new("content_excerpt_comma_machine_tokens");
    let logs_dir = root.path.join("logs");
    fs::create_dir_all(&logs_dir).expect("mkdir logs");
    let log = logs_dir.join("clawd.run.log");
    fs::write(&log, "INFO boot start\nINFO worker ready\n").expect("write clawd log");
    let logs_path = logs_dir.display().to_string();
    let log_path = log.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = logs_path.clone();
    route.output_contract.delivery_required = false;
    route.output_contract.requires_content_evidence = true;
    route.resolved_intent =
        "read logs/clawd.run.log last lines (slice_mode=tail, slice_n=20)".to_string();
    route.route_reason =
        "tail the last 20 lines of logs/clawd.run.log, then synthesize a one-sentence judgment"
            .to_string();

    let plan = content_excerpt_summary_directory_log_slice_deterministic_plan_result(
        "read clawd log tail",
        Some(&route),
        &LoopState::new(1),
        Some(&logs_path),
    )
    .expect("comma-delimited machine slice tokens should still produce deterministic plan");

    let read_action = plan.steps[1].to_agent_action().unwrap();
    let read_args = expect_planned_call(&read_action, "fs_basic", "read_text_range");
    assert_eq!(
        read_args.get("path").and_then(Value::as_str),
        Some(log_path.as_str())
    );
    assert_eq!(read_args.get("mode").and_then(Value::as_str), Some("tail"));
    assert_eq!(read_args.get("n").and_then(Value::as_u64), Some(20));
}

#[test]
fn generic_log_analyze_does_not_steal_directory_with_explicit_log_file_target() {
    let root = TempDirGuard::new("generic_log_analyze_skip_explicit_log_file");
    let logs_dir = root.path.join("logs");
    fs::create_dir_all(&logs_dir).expect("mkdir logs");
    let log = logs_dir.join("clawd.run.log");
    fs::write(&log, "INFO boot start\nINFO worker ready\n").expect("write clawd log");
    let logs_path = logs_dir.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = logs_path.clone();
    route.output_contract.delivery_required = false;
    route.output_contract.requires_content_evidence = true;
    route.resolved_intent =
        "find clawd logs and read logs/clawd.run.log before judgment".to_string();

    let target = generic_path_content_log_analyze_target_path(Some(&route), Some(&logs_path));

    assert!(
        target.is_none(),
        "directory-level log_analyze must not steal a request that names an exact log file under that directory"
    );
}

#[test]
fn explicit_document_targets_win_over_workspace_log_analyze() {
    let root = TempDirGuard::new("explicit_docs_before_workspace_logs");
    fs::write(
        root.path.join("clawd-runtime.log"),
        "ERROR old provider timeout\n",
    )
    .expect("write workspace log");
    fs::write(root.path.join("README.md"), "# Runtime\n\ncheckpoint_id\n").expect("write readme");
    let plan_dir = root.path.join("plan");
    fs::create_dir_all(&plan_dir).expect("create plan dir");
    fs::write(
        plan_dir.join("background_task_resume_convergence_plan_20260621.md"),
        "resume_entrypoint\nresume_work_item\nresume_executor\ndispatch_state\nresult_projection_state\n",
    )
    .expect("write plan");
    let plan_path = "plan/background_task_resume_convergence_plan_20260621.md";
    let mut state = test_state_with_enabled_skills(&["log_analyze", "fs_basic"]);
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.delivery_required = false;
    route.resolved_intent = format!("summarize README.md and {plan_path}; current_workspace_scope");
    let user_text =
        format!("Use README.md and {plan_path} to explain checkpoint_id and resume_entrypoint.");

    let plan = content_excerpt_explicit_file_targets_deterministic_plan_result(
        &state,
        "explain checkpoint/resume fields from explicit docs",
        Some(&route),
        &LoopState::new(1),
        &user_text,
        None,
        Some(root.path.to_string_lossy().as_ref()),
    )
    .expect("explicit document targets should produce bounded reads before log analysis");

    assert_eq!(plan.steps.len(), 4);
    let read_paths = plan
        .steps
        .iter()
        .filter_map(|step| step.to_agent_action())
        .filter_map(|action| match action {
            AgentAction::CallTool { tool, args }
                if tool == "fs_basic"
                    && args.get("action").and_then(Value::as_str) == Some("read_text_range") =>
            {
                args.get("path")
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(read_paths.len(), 2);
    assert!(read_paths.iter().any(|path| path.ends_with("README.md")));
    assert!(read_paths.iter().any(|path| path.ends_with(plan_path)));
    assert!(matches!(
        plan.steps[2].to_agent_action(),
        Some(AgentAction::SynthesizeAnswer { evidence_refs })
            if evidence_refs == vec!["step_1".to_string(), "step_2".to_string()]
    ));
}

#[test]
fn explicit_log_and_document_targets_are_both_read_before_synthesis() {
    let root = TempDirGuard::new("explicit_log_doc_targets");
    let logs_dir = root.path.join("logs");
    let docs_dir = root.path.join("docs");
    fs::create_dir_all(&logs_dir).expect("create logs dir");
    fs::create_dir_all(&docs_dir).expect("create docs dir");
    fs::write(
        logs_dir.join("app.log"),
        "WARN slow request\nERROR failed request\n",
    )
    .expect("write log");
    fs::write(
        docs_dir.join("service_notes.md"),
        "# Service Notes\n\nCheck logs first.\n",
    )
    .expect("write doc");
    let log_path = "logs/app.log";
    let doc_path = "docs/service_notes.md";
    let mut state = test_state_with_enabled_skills(&["fs_basic"]);
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = format!("{log_path} | {doc_path}");
    route.output_contract.delivery_required = false;
    route.resolved_intent = format!("summarize {log_path} and {doc_path}");
    let user_text = format!("Analyze {log_path}; parse {doc_path}; then synthesize.");

    let plan = content_excerpt_explicit_file_targets_deterministic_plan_result(
        &state,
        "summarize explicit log and doc targets",
        Some(&route),
        &LoopState::new(1),
        &user_text,
        None,
        Some(root.path.to_string_lossy().as_ref()),
    )
    .expect("explicit log and doc targets should produce bounded reads");

    assert_eq!(plan.steps.len(), 4);
    let read_paths = plan
        .steps
        .iter()
        .filter_map(|step| step.to_agent_action())
        .filter_map(|action| match action {
            AgentAction::CallTool { tool, args }
                if tool == "fs_basic"
                    && args.get("action").and_then(Value::as_str) == Some("read_text_range") =>
            {
                args.get("path")
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(read_paths.len(), 2);
    assert!(read_paths.iter().any(|path| path.ends_with(log_path)));
    assert!(read_paths.iter().any(|path| path.ends_with(doc_path)));
    assert!(matches!(
        plan.steps[2].to_agent_action(),
        Some(AgentAction::SynthesizeAnswer { evidence_refs })
            if evidence_refs == vec!["step_1".to_string(), "step_2".to_string()]
    ));
}

#[test]
fn explicit_raw_output_file_target_preserves_tail_slice_selector() {
    let root = TempDirGuard::new("explicit_raw_output_tail_slice");
    let logs_dir = root.path.join("logs");
    fs::create_dir_all(&logs_dir).expect("create logs dir");
    fs::write(
        logs_dir.join("act_plan.log"),
        "line1\nline2\nline3\nline4\nline5\n",
    )
    .expect("write log");
    let log_path = "logs/act_plan.log";
    let mut state = test_state_with_enabled_skills(&["fs_basic"]);
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = log_path.to_string();
    route.output_contract.delivery_required = false;
    route.resolved_intent = format!("{log_path} slice_mode=tail slice_n=3");
    let user_text = format!("read {log_path} tail slice");

    let plan = content_excerpt_explicit_file_targets_deterministic_plan_result(
        &state,
        "read the selected raw log tail",
        Some(&route),
        &LoopState::new(1),
        &user_text,
        None,
        Some(root.path.to_string_lossy().as_ref()),
    )
    .expect("explicit raw output target should produce a bounded read");

    let read_action = plan.steps[0]
        .to_agent_action()
        .expect("first step should be a read action");
    let read_args = expect_planned_call(&read_action, "fs_basic", "read_text_range");
    assert_eq!(
        read_args.get("path").and_then(Value::as_str),
        Some(root.path.join(log_path).to_string_lossy().as_ref())
    );
    assert_eq!(read_args.get("mode").and_then(Value::as_str), Some("tail"));
    assert_eq!(read_args.get("n").and_then(Value::as_u64), Some(3));
    assert!(matches!(
        plan.steps[1].to_agent_action(),
        Some(AgentAction::SynthesizeAnswer { evidence_refs })
            if evidence_refs == vec!["step_1".to_string()]
    ));
    assert!(matches!(
        plan.steps[2].to_agent_action(),
        Some(AgentAction::Respond { content }) if content == "{{last_output}}"
    ));
}

#[test]
fn explicit_file_target_reads_structured_update_slice_from_goal_context() {
    let root = TempDirGuard::new("explicit_goal_structured_slice");
    let logs_dir = root.path.join("logs");
    fs::create_dir_all(&logs_dir).expect("create logs dir");
    fs::write(logs_dir.join("act_plan.log"), "a\nb\nc\nd\ne\n").expect("write log");
    let log_path = "logs/act_plan.log";
    let mut state = test_state_with_enabled_skills(&["fs_basic"]);
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = log_path.to_string();
    route.output_contract.delivery_required = false;
    route.resolved_intent = log_path.to_string();
    route.route_reason = "raw_command_output contract for direct bounded line slice".to_string();
    let goal = format!(
        r#"Current task:
Structured update: {{"slice_mode":"tail","slice_n":3}}
Bound target: {log_path}"#
    );

    let plan = content_excerpt_explicit_file_targets_deterministic_plan_result(
        &state,
        &goal,
        Some(&route),
        &LoopState::new(1),
        log_path,
        None,
        Some(root.path.to_string_lossy().as_ref()),
    )
    .expect("explicit raw output target should consume structured slice tokens from goal context");

    let read_action = plan.steps[0]
        .to_agent_action()
        .expect("first step should be a read action");
    let read_args = expect_planned_call(&read_action, "fs_basic", "read_text_range");
    assert_eq!(read_args.get("mode").and_then(Value::as_str), Some("tail"));
    assert_eq!(read_args.get("n").and_then(Value::as_u64), Some(3));
}

#[test]
fn generic_single_document_synthesis_rewrites_bounded_read_to_doc_parse() {
    let root = TempDirGuard::new("generic_doc_parse_synthesis");
    let readme = root.path.join("README.md");
    fs::write(&readme, "# RustClaw\n\nA local agent runtime.").expect("write readme");
    let readme_path = readme.display().to_string();
    let state = test_state_with_enabled_skills(&["doc_parse", "fs_basic"]);
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "README.md".to_string();
    route.output_contract.delivery_required = false;
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": readme_path.clone(),
                "mode": "head",
                "n": 20
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
        &state,
        Some(&route),
        &LoopState::new(1),
        "parse README and summarize the key points",
        None,
        actions,
    );

    assert!(matches!(
        &normalized[0],
        AgentAction::CallSkill { skill, args }
            if skill == "doc_parse"
                && args.get("action").and_then(Value::as_str) == Some("parse_doc")
                && args.get("path").and_then(Value::as_str) == Some(readme_path.as_str())
    ));
    assert!(matches!(
        normalized.get(1),
        Some(AgentAction::SynthesizeAnswer { .. })
    ));
}

#[test]
fn excerpt_kind_mixed_field_and_read_plan_preserves_config_field_reads() {
    let root = TempDirGuard::new("excerpt_kind_mixed_field_read");
    let cargo = root.path.join("Cargo.toml");
    let ui_dir = root.path.join("UI");
    let package = ui_dir.join("package.json");
    let readme = root.path.join("README.md");
    fs::create_dir_all(&ui_dir).expect("create ui dir");
    fs::write(&cargo, "[package]\nname = \"clawd\"\n").expect("write cargo");
    fs::write(&package, r#"{"name":"react-example"}"#).expect("write package");
    fs::write(&readme, "# RustClaw\n\nA local Rust agent runtime with UI.").expect("write readme");
    let cargo_path = cargo.display().to_string();
    let package_path = package.display().to_string();
    let readme_path = readme.display().to_string();
    let state = test_state_with_enabled_skills(&["config_basic", "doc_parse", "fs_basic"]);
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::ExcerptKindJudgment;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = format!("{cargo_path}|{package_path}|{readme_path}");
    route.output_contract.delivery_required = false;
    let actions = vec![
        AgentAction::CallTool {
            tool: "config_basic".to_string(),
            args: json!({
                "action": "read_field",
                "path": cargo_path.clone(),
                "field_path": "package.name"
            }),
        },
        AgentAction::CallTool {
            tool: "config_basic".to_string(),
            args: json!({
                "action": "read_field",
                "path": package_path.clone(),
                "field_path": "name"
            }),
        },
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": readme_path.clone(),
                "mode": "head",
                "n": 30
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec![
                "step_1".to_string(),
                "step_2".to_string(),
                "step_3".to_string(),
            ],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = super::super::normalize_planned_actions(
        &state,
        Some(&route),
        &LoopState::new(1),
        "read package fields and README, then classify the project shape",
        None,
        actions,
    );

    let first_args = expect_planned_call(&normalized[0], "config_basic", "read_field");
    assert_eq!(
        first_args.get("path").and_then(Value::as_str),
        Some(cargo_path.as_str())
    );
    assert_eq!(
        first_args.get("field_path").and_then(Value::as_str),
        Some("package.name")
    );
    let second_args = expect_planned_call(&normalized[1], "config_basic", "read_field");
    assert_eq!(
        second_args.get("path").and_then(Value::as_str),
        Some(package_path.as_str())
    );
    assert_eq!(
        second_args.get("field_path").and_then(Value::as_str),
        Some("name")
    );
    assert!(matches!(
        &normalized[2],
        AgentAction::CallSkill { skill, args }
            if skill == "doc_parse"
                && args.get("path").and_then(Value::as_str) == Some(readme_path.as_str())
    ));
}

#[test]
fn content_excerpt_with_summary_rewrites_bounded_read_to_doc_parse() {
    let root = TempDirGuard::new("content_excerpt_with_summary_doc_parse");
    let readme = root.path.join("README.md");
    fs::write(&readme, "# RustClaw\n\nA local agent runtime.").expect("write readme");
    let readme_path = readme.display().to_string();
    let state = test_state_with_enabled_skills(&["doc_parse", "fs_basic"]);
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptWithSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = readme_path.clone();
    route.output_contract.delivery_required = false;
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": readme_path.clone(),
                "mode": "head",
                "n": 80
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
        &state,
        Some(&route),
        &LoopState::new(1),
        "parse the document and summarize the key points",
        None,
        actions,
    );

    assert!(matches!(
        &normalized[0],
        AgentAction::CallSkill { skill, args }
            if skill == "doc_parse"
                && args.get("action").and_then(Value::as_str) == Some("parse_doc")
                && args.get("path").and_then(Value::as_str) == Some(readme_path.as_str())
    ));
    assert!(matches!(
        normalized.get(1),
        Some(AgentAction::SynthesizeAnswer { .. })
    ));
}

#[test]
fn workspace_project_summary_keeps_contract_allowed_bounded_read() {
    let root = TempDirGuard::new("workspace_project_summary_bounded_read");
    let readme = root.path.join("README.md");
    fs::write(&readme, "# RustClaw\n\nA local agent runtime.").expect("write readme");
    let readme_path = readme.display().to_string();
    let state = test_state_with_enabled_skills(&["doc_parse", "fs_basic"]);
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::WorkspaceProjectSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = readme_path.clone();
    route.output_contract.delivery_required = false;
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": readme_path.clone(),
                "mode": "head",
                "n": 30
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
        &state,
        Some(&route),
        &LoopState::new(2),
        "read README and summarize the project",
        None,
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "read_text_range");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(readme_path.as_str())
    );
    assert!(matches!(
        normalized.get(1),
        Some(AgentAction::SynthesizeAnswer { .. })
    ));
}

#[test]
fn generic_single_log_synthesis_rewrites_bounded_read_to_log_analyze() {
    let root = TempDirGuard::new("generic_log_analyze_synthesis");
    let log = root.path.join("app.log");
    fs::write(
        &log,
        "INFO boot ok\nWARN latency high\nERROR provider timeout\nINFO retry ok\n",
    )
    .expect("write log");
    let log_path = log.display().to_string();
    let state = test_state_with_enabled_skills(&["log_analyze", "fs_basic"]);
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = log_path.clone();
    route.output_contract.delivery_required = false;
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": log_path.clone(),
                "mode": "head",
                "n": 20
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
        &state,
        Some(&route),
        &LoopState::new(1),
        "analyze this log briefly",
        None,
        actions,
    );

    assert!(matches!(
        &normalized[0],
        AgentAction::CallSkill { skill, args }
            if skill == "log_analyze"
                && args.get("path").and_then(Value::as_str) == Some(log_path.as_str())
                && args.get("max_matches").and_then(Value::as_u64) == Some(50)
    ));
    assert!(matches!(
        normalized.get(1),
        Some(AgentAction::SynthesizeAnswer { .. })
    ));
}

#[test]
fn generic_single_log_synthesis_preserves_tail_read_range() {
    let root = TempDirGuard::new("generic_log_tail_read_preserved");
    let log = root.path.join("app.log");
    fs::write(
        &log,
        "INFO boot ok\nWARN latency high\nERROR provider timeout\nINFO retry ok\n",
    )
    .expect("write log");
    let log_path = log.display().to_string();
    let state = test_state_with_enabled_skills(&["log_analyze", "fs_basic"]);
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = log_path.clone();
    route.output_contract.delivery_required = false;
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": log_path.clone(),
                "mode": "tail",
                "n": 20
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
        &state,
        Some(&route),
        &LoopState::new(1),
        "summarize this log tail",
        None,
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "read_text_range");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some(log_path.as_str())
    );
    assert_eq!(args.get("mode").and_then(Value::as_str), Some("tail"));
    assert_eq!(args.get("n").and_then(Value::as_u64), Some(20));
    assert!(matches!(
        normalized.get(1),
        Some(AgentAction::SynthesizeAnswer { .. })
    ));
}

#[test]
fn generic_log_directory_auto_locator_uses_log_analyze_plan() {
    let root = TempDirGuard::new("generic_log_directory_auto_locator");
    let logs_dir = root.path.join("logs");
    fs::create_dir_all(&logs_dir).expect("mkdir logs");
    fs::write(
        logs_dir.join("app.log"),
        "INFO boot ok\nWARN latency high\nERROR provider timeout\n",
    )
    .expect("write log");
    fs::write(logs_dir.join("notes.txt"), "not a log").expect("write notes");
    let logs_path = logs_dir.display().to_string();
    let state = test_state_with_enabled_skills(&["log_analyze", "fs_basic"]);
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = logs_path.clone();
    route.output_contract.delivery_required = false;

    let plan = generic_path_content_log_analyze_deterministic_plan_result(
        "inspect the current target",
        &state,
        Some(&route),
        &LoopState::new(1),
        Some(&logs_path),
    )
    .expect("log analyze plan");

    assert_eq!(plan.steps.len(), 3);
    assert_eq!(plan.steps[0].action_type, "call_skill");
    assert_eq!(plan.steps[0].skill, "log_analyze");
    assert_eq!(
        plan.steps[0].args.get("path").and_then(Value::as_str),
        Some(logs_path.as_str())
    );
    assert_eq!(
        plan.steps[0]
            .args
            .get("max_matches")
            .and_then(Value::as_u64),
        Some(50)
    );
    assert_eq!(plan.steps[1].action_type, "synthesize_answer");
}

#[test]
fn content_excerpt_summary_log_directory_auto_locator_uses_log_analyze_plan() {
    let root = TempDirGuard::new("content_excerpt_summary_log_directory_auto_locator");
    let logs_dir = root.path.join("logs");
    fs::create_dir_all(&logs_dir).expect("mkdir logs");
    fs::write(
        logs_dir.join("model_io.log"),
        "INFO request ok\nWARN slow provider\nERROR verifier timeout\n",
    )
    .expect("write log");
    let logs_path = logs_dir.display().to_string();
    let state = test_state_with_enabled_skills(&["log_analyze", "fs_basic"]);
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = logs_path.clone();
    route.output_contract.delivery_required = false;

    let plan = generic_path_content_log_analyze_deterministic_plan_result(
        "inspect the current target",
        &state,
        Some(&route),
        &LoopState::new(1),
        Some(&logs_path),
    )
    .expect("log analyze plan");

    assert_eq!(plan.steps.len(), 3);
    assert_eq!(plan.steps[0].action_type, "call_skill");
    assert_eq!(plan.steps[0].skill, "log_analyze");
    assert_eq!(
        plan.steps[0].args.get("path").and_then(Value::as_str),
        Some(logs_path.as_str())
    );
}

#[test]
fn content_excerpt_summary_single_log_file_without_slice_defers_to_log_analyze_plan() {
    let root = TempDirGuard::new("content_excerpt_summary_single_log_file_no_slice");
    let logs_dir = root.path.join("logs");
    fs::create_dir_all(&logs_dir).expect("mkdir logs");
    let log = logs_dir.join("app.log");
    fs::write(&log, "INFO ok\nWARN slow provider\nERROR timeout\n").expect("write log");
    let log_path = log.display().to_string();
    let state = test_state_with_enabled_skills(&["log_analyze", "fs_basic"]);
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = log_path.clone();
    route.output_contract.delivery_required = false;

    assert!(
        content_excerpt_explicit_file_targets_deterministic_plan_result(
            &state,
            "inspect the log target",
            Some(&route),
            &LoopState::new(1),
            "",
            None,
            Some(&log_path),
        )
        .is_none()
    );

    let plan = generic_path_content_log_analyze_deterministic_plan_result(
        "inspect the log target",
        &state,
        Some(&route),
        &LoopState::new(1),
        Some(&log_path),
    )
    .expect("single log summary should use log_analyze");

    assert_eq!(plan.steps.len(), 3);
    assert_eq!(plan.steps[0].action_type, "call_skill");
    assert_eq!(plan.steps[0].skill, "log_analyze");
    assert_eq!(
        plan.steps[0].args.get("path").and_then(Value::as_str),
        Some(log_path.as_str())
    );
}

#[test]
fn content_excerpt_single_doc_file_without_slice_uses_doc_parse_plan() {
    let root = TempDirGuard::new("content_excerpt_summary_single_doc_file_no_slice");
    let readme = root.path.join("README.md");
    fs::write(&readme, "# RustClaw\n\nLocal agent runtime.\n").expect("write readme");
    let readme_path = readme.display().to_string();
    let state = test_state_with_enabled_skills(&["doc_parse", "fs_basic"]);
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptWithSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = readme_path.clone();
    route.output_contract.delivery_required = false;

    let plan = content_excerpt_explicit_file_targets_deterministic_plan_result(
        &state,
        "summarize the resolved document",
        Some(&route),
        &LoopState::new(1),
        "",
        None,
        Some(&readme_path),
    )
    .expect("single document summary should use doc_parse");

    assert_eq!(plan.steps.len(), 3);
    assert_eq!(plan.steps[0].action_type, "call_skill");
    assert_eq!(plan.steps[0].skill, "doc_parse");
    assert_eq!(
        plan.steps[0].args.get("action").and_then(Value::as_str),
        Some("parse_doc")
    );
    assert_eq!(
        plan.steps[0].args.get("path").and_then(Value::as_str),
        Some(readme_path.as_str())
    );
}

#[test]
fn content_excerpt_summary_keeps_bounded_log_read_for_synthesis() {
    let root = TempDirGuard::new("content_excerpt_log_read_synthesis");
    let log = root.path.join("model_io.log");
    fs::write(
        &log,
        "INFO boot ok\nWARN latency high\nERROR provider timeout\nINFO retry ok\n",
    )
    .expect("write log");
    let log_path = log.display().to_string();
    let state = test_state_with_enabled_skills(&["log_analyze", "fs_basic"]);
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = log_path.clone();
    route.output_contract.delivery_required = false;
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": log_path.clone(),
                "mode": "tail",
                "n": 4
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
        &state,
        Some(&route),
        &LoopState::new(1),
        "summarize the last log lines",
        None,
        actions,
    );

    assert!(matches!(
        &normalized[0],
        AgentAction::CallTool { tool, args }
            if tool == "fs_basic"
                && args.get("action").and_then(Value::as_str) == Some("read_text_range")
                && args.get("path").and_then(Value::as_str) == Some(log_path.as_str())
                && args.get("mode").and_then(Value::as_str) == Some("tail")
    ));
    assert!(matches!(
        normalized.get(1),
        Some(AgentAction::SynthesizeAnswer { .. })
    ));
}

#[test]
fn content_excerpt_contract_rewrites_concrete_respond_after_synthesis() {
    let state = test_state_with_registry();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptWithSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.last_output = Some("observed tail evidence".to_string());
    let actions = vec![
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "stale concrete summary".to_string(),
        },
    ];

    let normalized = super::super::normalize_planned_actions(
        &state,
        Some(&route),
        &loop_state,
        "summarize observed excerpt",
        None,
        actions,
    );

    assert!(matches!(
        normalized.last(),
        Some(AgentAction::Respond { content }) if content == "{{last_output}}"
    ));
}

#[test]
fn content_excerpt_summary_auto_locator_deterministic_plan_uses_fs_basic_for_repo_prompt_doc() {
    let root = TempDirGuard::new("content_excerpt_repo_prompt_deterministic_plan");
    let prompt_dir = root.path.join("prompts/layers/generated/skills");
    fs::create_dir_all(&prompt_dir).expect("create prompt dir");
    let prompt_file = prompt_dir.join("fs_basic.md");
    fs::write(
        &prompt_file,
        "## fs_basic\n\nFilesystem facts and bounded reads.",
    )
    .expect("write prompt file");
    let prompt_path = prompt_file.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;
    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.join("workspace_root");

    let plan = content_excerpt_summary_auto_locator_deterministic_plan_result(
        &state,
        "summarize a generated skill prompt",
        Some(&route),
        &loop_state,
        Some(&prompt_path),
    )
    .expect("repo prompt artifact should use a bounded filesystem read");

    assert_eq!(plan.plan_kind, PlanKind::Single);
    assert_eq!(plan.steps.len(), 3);
    match &plan.steps[0].to_agent_action() {
        Some(AgentAction::CallTool { tool, args }) => {
            assert_eq!(tool, "fs_basic");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("read_text_range")
            );
            assert_eq!(
                args.get("path").and_then(Value::as_str),
                Some(prompt_path.as_str())
            );
        }
        other => panic!("expected fs_basic read_text_range action, got {other:?}"),
    }
    assert!(matches!(
        plan.steps[1].to_agent_action(),
        Some(AgentAction::SynthesizeAnswer { evidence_refs }) if evidence_refs == vec!["last_output".to_string()]
    ));
    assert!(matches!(
        plan.steps[2].to_agent_action(),
        Some(AgentAction::Respond { content }) if content == "{{last_output}}"
    ));
}

#[test]
fn excerpt_kind_judgment_resolved_file_path_uses_bounded_read_and_synthesis() {
    let root = TempDirGuard::new("excerpt_kind_judgment_resolved_path");
    let logs_dir = root.path.join("logs");
    fs::create_dir_all(&logs_dir).expect("create logs dir");
    let log_file = logs_dir.join("clawd.codex.minimax.log");
    fs::write(
        &log_file,
        "2026-06-17T04:06:49Z INFO task_call phase=failure\n{\"kind\":\"ask\",\"summary\":{\"final_status\":\"failed\"}}\n",
    )
    .expect("write log");
    let log_path = log_file.display().to_string();
    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        false,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ExcerptKindJudgment;
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "clawd.codex.minimax.log".to_string();
    route.output_contract.delivery_required = false;
    route.output_contract.requires_content_evidence = false;
    route.resolved_intent =
        "Classify the bound file from logs/clawd.codex.minimax.log using bounded content evidence."
            .to_string();
    route.route_reason = "existing_observed_context_synthesis".to_string();
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let plan = content_excerpt_summary_auto_locator_deterministic_plan_result(
        &state,
        "classify the bound file content",
        Some(&route),
        &loop_state,
        None,
    )
    .expect("excerpt kind judgment should read the resolved file before synthesis");

    assert_eq!(plan.plan_kind, PlanKind::Single);
    assert_eq!(plan.steps.len(), 3);
    let read_action = plan.steps[0].to_agent_action().expect("read action");
    let read_args = expect_planned_call(&read_action, "fs_basic", "read_text_range");
    assert_eq!(
        read_args.get("path").and_then(Value::as_str),
        Some(log_path.as_str())
    );
    assert_eq!(read_args.get("mode").and_then(Value::as_str), Some("head"));
    assert_eq!(read_args.get("n").and_then(Value::as_u64), Some(80));
    assert!(matches!(
        plan.steps[1].to_agent_action(),
        Some(AgentAction::SynthesizeAnswer { evidence_refs }) if evidence_refs == vec!["last_output".to_string()]
    ));
    assert!(matches!(
        plan.steps[2].to_agent_action(),
        Some(AgentAction::Respond { content }) if content == "{{last_output}}"
    ));
}

#[test]
fn content_excerpt_with_summary_does_not_use_head_read_deterministic_plan() {
    let root = TempDirGuard::new("content_excerpt_with_summary_no_deterministic_plan");
    let log = root.path.join("model_io.log");
    fs::write(&log, "line 1\nline 2\nline 3\nline 4\n").expect("write log");
    let log_path = log.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptWithSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    assert!(
        content_excerpt_summary_auto_locator_deterministic_plan_result(
            &test_state(),
            "show a bounded excerpt and summarize it",
            Some(&route),
            &loop_state,
            Some(&log_path),
        )
        .is_none()
    );
}

#[test]
fn scalar_content_auto_locator_skips_content_excerpt_with_summary_contract() {
    let root = TempDirGuard::new("scalar_content_auto_locator_skips_content_excerpt");
    let log = root.path.join("model_io.log");
    fs::write(&log, "line 1\nline 2\nline 3\nline 4\n").expect("write log");
    let log_path = log.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptWithSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;
    let state = test_state();

    assert!(scalar_content_auto_locator_deterministic_plan_result(
        &state,
        "show a bounded excerpt and summarize it",
        Some(&route),
        &loop_state,
        "show the last 4 lines and summarize recovery status",
        Some("show the last 4 lines and summarize recovery status"),
        Some(&log_path),
    )
    .is_none());
}

#[test]
fn generic_content_evidence_does_not_use_single_file_deterministic_plan() {
    let root = TempDirGuard::new("generic_content_evidence_no_deterministic_plan");
    let readme = root.path.join("README.md");
    fs::write(&readme, "# RustClaw\n\nA local agent runtime.").expect("write readme");
    let readme_path = readme.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.delivery_required = false;
    route.output_contract.requires_content_evidence = true;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    assert!(
        content_excerpt_summary_auto_locator_deterministic_plan_result(
            &test_state(),
            "summarize a resolved local document",
            Some(&route),
            &loop_state,
            Some(&readme_path),
        )
        .is_none()
    );
}

#[test]
fn structured_scalar_compare_does_not_use_single_file_content_deterministic_plan() {
    let root = TempDirGuard::new("structured_scalar_no_single_content_deterministic_plan");
    let readme = root.path.join("README.md");
    fs::write(&readme, "# RustClaw\n\nA local agent runtime.").expect("write readme");
    let readme_path = readme.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "README.md | AGENTS.md".to_string();
    route.output_contract.delivery_required = false;
    route.output_contract.requires_content_evidence = true;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    assert!(
        content_excerpt_summary_auto_locator_deterministic_plan_result(
            &test_state(),
            "compare files",
            Some(&route),
            &loop_state,
            Some(&readme_path),
        )
        .is_none()
    );
}

#[test]
fn scalar_content_auto_locator_does_not_read_path_only_contract() {
    let root = TempDirGuard::new("scalar_content_auto_locator");
    let note = root.path.join("service_notes.md");
    fs::write(&note, "# Reading Notes\n\nService status is healthy.").expect("write note");
    let note_path = note.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;
    let state = test_state();

    assert!(scalar_content_auto_locator_deterministic_plan_result(
        &state,
        "extract scalar from resolved file content",
        Some(&route),
        &loop_state,
        "extract scalar from resolved file content",
        Some("extract scalar from resolved file content"),
        Some(&note_path),
    )
    .is_none());
}

#[test]
fn scalar_content_auto_locator_does_not_read_generated_file_path_report_target() {
    let root = TempDirGuard::new("scalar_content_auto_locator_generated_path");
    let image = root.path.join("document").join("skill_audio_smoke.mp3");
    fs::create_dir_all(image.parent().expect("image parent")).expect("create document dir");
    fs::write(&image, b"existing media bytes").expect("write existing media");
    let image_path = image.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::GeneratedFilePathReport;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_hint = image_path.clone();
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;
    let state = test_state();

    assert!(scalar_content_auto_locator_deterministic_plan_result(
        &state,
        "generate a media artifact and return the saved path",
        Some(&route),
        &loop_state,
        "generate a media artifact and return the saved path",
        Some("generate a media artifact and return the saved path"),
        Some(&image_path),
    )
    .is_none());
}

#[test]
fn scalar_content_auto_locator_does_not_read_existence_contract() {
    let root = TempDirGuard::new("scalar_content_auto_locator_existence");
    let note = root.path.join("package.json");
    fs::write(&note, r#"{"name":"fixture"}"#).expect("write package");
    let note_path = note.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;
    let state = test_state();

    assert!(scalar_content_auto_locator_deterministic_plan_result(
        &state,
        "check whether the file exists",
        Some(&route),
        &loop_state,
        "check whether the file exists",
        Some("check whether the file exists"),
        Some(&note_path),
    )
    .is_none());
}

#[test]
fn scalar_content_auto_locator_reads_generic_scalar_content_contract() {
    let root = TempDirGuard::new("scalar_content_auto_locator_generic");
    let note = root.path.join("service_notes.md");
    fs::write(&note, "# Reading Notes\n\nService status is healthy.").expect("write note");
    let note_path = note.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;
    let state = test_state();

    let plan = scalar_content_auto_locator_deterministic_plan_result(
        &state,
        "extract scalar from resolved file content",
        Some(&route),
        &loop_state,
        "extract scalar from resolved file content",
        Some("extract scalar from resolved file content"),
        Some(&note_path),
    )
    .expect("generic content-evidence scalar contracts should read the resolved file");

    assert_eq!(plan.steps.len(), 3);
    assert!(matches!(
        plan.steps[0].to_agent_action(),
        Some(AgentAction::CallTool { ref tool, ref args })
            if tool == "fs_basic"
                && args.get("action").and_then(Value::as_str) == Some("read_text_range")
                && args.get("path").and_then(Value::as_str) == Some(note_path.as_str())
    ));
    assert!(matches!(
        plan.steps[1].to_agent_action(),
        Some(AgentAction::SynthesizeAnswer { .. })
    ));
}

#[test]
fn scalar_content_auto_locator_validates_config_contract() {
    let root = TempDirGuard::new("scalar_content_auto_locator_config_validation");
    let config = root.path.join("config.toml");
    fs::write(&config, "[service]\nname = \"rustclaw\"\n").expect("write config");
    let config_path = config.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ConfigValidation;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = config_path.clone();
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;
    let state = test_state();

    let plan = scalar_content_auto_locator_deterministic_plan_result(
        &state,
        "validate structured config syntax",
        Some(&route),
        &loop_state,
        "validate structured config syntax",
        Some("validate structured config syntax"),
        Some(&config_path),
    )
    .expect("config validation should use structured validation");

    assert_eq!(plan.steps.len(), 1);
    assert!(matches!(
        plan.steps[0].to_agent_action(),
        Some(AgentAction::CallTool { ref tool, ref args })
            if tool == "config_basic"
                && args.get("action").and_then(Value::as_str) == Some("validate")
                && args.get("path").and_then(Value::as_str) == Some(config_path.as_str())
                && args.get("validation_profile").and_then(Value::as_str)
                    == Some("syntax_only")
    ));
}

#[test]
fn scalar_content_auto_locator_uses_structured_read_field_for_structured_scalar_contract() {
    let root = TempDirGuard::new("scalar_content_auto_locator_structured_field");
    let manifest = root.path.join("Cargo.toml");
    fs::write(&manifest, "[package]\nname = \"rustclaw-test\"\n").expect("write manifest");
    let manifest_path = manifest.display().to_string();
    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = manifest_path.clone();
    route.resolved_intent =
        "Read package.name from Cargo.toml and output only that value.".to_string();
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let plan = scalar_content_auto_locator_deterministic_plan_result(
        &state,
        "Read package.name from Cargo.toml and output only that value.",
        Some(&route),
        &loop_state,
        "Read package.name from Cargo.toml and output only that value.",
        Some("Read package.name from Cargo.toml and output only that value."),
        Some(&manifest_path),
    )
    .expect("structured scalar contracts should use structured field reads");

    assert_eq!(plan.steps.len(), 1);
    let actual = plan.steps[0].to_agent_action();
    assert!(
        matches!(
        actual,
        Some(AgentAction::CallTool { ref tool, ref args })
            if tool == "config_basic"
                && args.get("action").and_then(Value::as_str) == Some("read_field")
                && args.get("path").and_then(Value::as_str) == Some(manifest_path.as_str())
                && args.get("field_path").and_then(Value::as_str) == Some("package.name")
        ),
        "unexpected plan action: {:?}",
        actual
    );
}

#[test]
fn scalar_content_auto_locator_preserves_explicit_member_manifest_package_version() {
    let root = TempDirGuard::new("scalar_content_auto_locator_workspace_version");
    fs::write(
        root.path.join("Cargo.toml"),
        r#"[workspace]
members = ["crates/clawd"]

[workspace.package]
version = "0.1.7"
"#,
    )
    .expect("write workspace manifest");
    let member_dir = root.path.join("crates/clawd");
    fs::create_dir_all(&member_dir).expect("create member");
    fs::write(
        member_dir.join("Cargo.toml"),
        r#"[package]
name = "clawd"
version.workspace = true
"#,
    )
    .expect("write member manifest");
    let member_manifest = member_dir.join("Cargo.toml");
    let member_path = member_manifest.display().to_string();
    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = member_path.clone();
    route.resolved_intent =
        "Read package.version from crates/clawd/Cargo.toml and output only the value.".to_string();
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;

    let plan = scalar_content_auto_locator_deterministic_plan_result(
        &state,
        "Read package.version from crates/clawd/Cargo.toml and output only the value.",
        Some(&route),
        &loop_state,
        "Read package.version from crates/clawd/Cargo.toml and output only the value.",
        Some("Read package.version from crates/clawd/Cargo.toml and output only the value."),
        Some(&member_path),
    )
    .expect("explicit member Cargo scalar contracts should read the member package field");

    assert_eq!(plan.steps.len(), 1);
    let actual = plan.steps[0].to_agent_action();
    assert!(
        matches!(
            actual,
        Some(AgentAction::CallTool { ref tool, ref args })
            if tool == "config_basic"
                && args.get("action").and_then(Value::as_str) == Some("read_field")
                && args.get("path").and_then(Value::as_str) == Some(member_path.as_str())
                && args.get("field_path").and_then(Value::as_str)
                    == Some("package.version")
        ),
        "unexpected plan action: {:?}",
        actual
    );
}

#[test]
fn scalar_content_auto_locator_ignores_memory_field_when_current_request_names_bare_key() {
    let root = TempDirGuard::new("scalar_content_auto_locator_bare_key");
    let fixture_dir = root.path.join("scripts/nl_tests/fixtures/device_local");
    fs::create_dir_all(&fixture_dir).expect("create fixture dir");
    let package = fixture_dir.join("package.json");
    fs::write(
        &package,
        r#"{
  "name": "rustclaw-nl-fixture",
  "version": "1.0.0",
  "scripts": { "build": "echo build" }
}"#,
    )
    .expect("write package");
    let package_path = package.display().to_string();
    let mut state = test_state();
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = package_path.clone();
    route.resolved_intent =
            "Extract the name field from scripts/nl_tests/fixtures/device_local/package.json and output only the value."
                .to_string();
    let mut loop_state = LoopState::default();
    loop_state.round_no = 1;
    let current_request =
        "读取 scripts/nl_tests/fixtures/device_local/package.json 的 name 字段，只输出值。";
    let goal = format!(
            "### PLANNER_MEMORY_CONTEXT\nfixture fact: scripts.build='echo build'\n\n### CURRENT_REQUEST\n{current_request}"
        );

    let plan = scalar_content_auto_locator_deterministic_plan_result(
        &state,
        &goal,
        Some(&route),
        &loop_state,
        current_request,
        Some(current_request),
        Some(&package_path),
    )
    .expect("bare schema key should be selected from current request");

    assert_eq!(plan.steps.len(), 1);
    assert!(matches!(
        plan.steps[0].to_agent_action(),
        Some(AgentAction::CallTool { ref tool, ref args })
            if tool == "config_basic"
                && args.get("action").and_then(Value::as_str) == Some("read_field")
                && args.get("path").and_then(Value::as_str) == Some(package_path.as_str())
                && args.get("field_path").and_then(Value::as_str) == Some("name")
    ));
}

#[test]
fn scalar_path_respond_only_uses_loop_state_auto_locator_observation() {
    let root = TempDirGuard::new("scalar_auto_locator_loop_state");
    let report = root.path.join("Report.MD");
    fs::write(&report, "hello").expect("write report");
    let report_path = report.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    route.output_contract.delivery_required = false;
    let actions = vec![AgentAction::Respond {
        content: report_path.clone(),
    }];
    let mut loop_state = LoopState::new(1);
    loop_state
        .output_vars
        .insert("auto_locator_path".to_string(), report_path.clone());

    let normalized = replace_scalar_path_respond_only_with_auto_locator_observation(
        Some(&route),
        &loop_state,
        None,
        actions,
    );
    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            assert_eq!(skill, "fs_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("stat_paths")
            );
            assert_eq!(args.get("paths"), Some(&json!([report_path])));
        }
        other => panic!("expected fs_basic stat_paths action, got {other:?}"),
    }
}

#[test]
fn scalar_count_synthesis_only_uses_count_inventory_for_locator_dir() {
    let root = TempDirGuard::new("scalar_count_locator_dir");
    fs::write(root.path.join("a.txt"), "a").expect("write a");
    fs::write(root.path.join("b.txt"), "b").expect("write b");
    fs::create_dir_all(root.path.join("child")).expect("create child");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_path.clone();
    route.output_contract.delivery_required = false;
    let actions = vec![
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = replace_scalar_count_plan_with_count_inventory(
        Some(&route),
        &LoopState::new(1),
        Some(&root_path),
        "count entries",
        actions,
    );

    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallTool { tool: skill, args } => {
            assert_eq!(skill, "fs_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("count_entries")
            );
            assert_eq!(
                args.get("path").and_then(|value| value.as_str()),
                Some(root_path.as_str())
            );
        }
        other => panic!("expected fs_basic count_entries action, got {other:?}"),
    }
}

#[test]
fn scalar_count_listing_plan_uses_count_inventory_for_locator_dir() {
    let root = TempDirGuard::new("scalar_count_listing_locator_dir");
    fs::write(root.path.join("a.txt"), "a").expect("write a");
    fs::write(root.path.join("b.txt"), "b").expect("write b");
    fs::create_dir_all(root.path.join("child")).expect("create child");
    let root_path = root.path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::Scalar,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = root_path.clone();
    route.output_contract.delivery_required = false;
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({"action": "count_entries", "path": root_path.clone()}),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = replace_scalar_count_plan_with_count_inventory(
        Some(&route),
        &LoopState::new(1),
        Some(&root_path),
        "count entries",
        actions,
    );

    assert_eq!(normalized.len(), 1);
    match &normalized[0] {
        AgentAction::CallTool { tool: skill, args } => {
            assert_eq!(skill, "fs_basic");
            assert_eq!(
                args.get("action").and_then(|value| value.as_str()),
                Some("count_entries")
            );
            assert_eq!(
                args.get("path").and_then(|value| value.as_str()),
                Some(root_path.as_str())
            );
        }
        other => panic!("expected fs_basic count_entries action, got {other:?}"),
    }
}

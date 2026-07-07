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
        crate::AskMode::act_with_chat_finalizer(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.delivery_required = false;
    route.output_contract.requires_content_evidence = true;
    route.route_reason = "capability_ref=filesystem.read_text_range".to_string();
    route.resolved_intent = "Summarize bounded file excerpt. slice_mode=tail slice_n=5".to_string();

    let read_args = assert_planner_supplied_tool_call_preserved(
        &state,
        &route,
        &LoopState::new(1),
        "read the requested bounded file excerpt and summarize it",
        Some("Summarize bounded file excerpt. slice_mode=tail slice_n=5"),
        Some(&config_path),
        "fs_basic",
        "read_text_range",
        json!({
            "action": "read_text_range",
            "path": config_path.clone(),
            "mode": "tail",
            "n": 5,
        }),
    );
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
        crate::AskMode::act_with_chat_finalizer(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.delivery_required = false;
    route.output_contract.requires_content_evidence = true;
    route.resolved_intent =
        "Summarize a bounded file excerpt without machine slice metadata.".to_string();

    let normalized = normalize_planned_actions_with_original_and_context(
        &state,
        Some(&route),
        &LoopState::new(1),
        "read an excerpt and summarize it",
        Some("Summarize a bounded file excerpt without machine slice metadata."),
        Some(&config_path),
        None,
        Vec::new(),
    );

    assert!(
        normalized.is_empty(),
        "content-excerpt route must not synthesize a default head range without planner action"
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
    let mut state = test_state_with_enabled_skills(&["fs_basic"]);
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::act_with_chat_finalizer(),
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

    route.route_reason = "capability_ref=filesystem.find_entries".to_string();
    let find_args = assert_planner_supplied_tool_call_preserved(
        &state,
        &route,
        &LoopState::new(1),
        "read the selected log tail and synthesize a one-sentence status judgment",
        Some(&logs_path),
        Some(&route.route_reason),
        "fs_basic",
        "find_entries",
        json!({
            "action": "find_entries",
            "root": logs_path.clone(),
            "pattern": "clawd",
            "target_kind": "file",
        }),
    );
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
    route.route_reason = "capability_ref=filesystem.read_text_range".to_string();
    let read_args = assert_planner_supplied_tool_call_preserved(
        &state,
        &route,
        &LoopState::new(1),
        "read the selected log tail and synthesize a one-sentence status judgment",
        Some(&logs_path),
        Some(&route.route_reason),
        "fs_basic",
        "read_text_range",
        json!({
            "action": "read_text_range",
            "path": log_path.clone(),
            "mode": "tail",
            "n": 20,
        }),
    );
    assert_eq!(
        read_args.get("path").and_then(Value::as_str),
        Some(log_path.as_str())
    );
    assert_eq!(read_args.get("mode").and_then(Value::as_str), Some("tail"));
    assert_eq!(read_args.get("n").and_then(Value::as_u64), Some(20));
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
    let mut state = test_state_with_enabled_skills(&["fs_basic"]);
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::act_with_chat_finalizer(),
        true,
        OutputResponseShape::Free,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptWithSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = logs_path.clone();
    route.output_contract.delivery_required = false;
    route.output_contract.requires_content_evidence = true;
    route.resolved_intent = "target_path=clawd.run.log slice_mode=tail slice_n=20".to_string();

    route.route_reason = "capability_ref=filesystem.find_entries".to_string();
    let find_args = assert_planner_supplied_tool_call_preserved(
        &state,
        &route,
        &LoopState::new(1),
        "read the selected log tail and synthesize status",
        Some(&logs_path),
        Some(&route.route_reason),
        "fs_basic",
        "find_entries",
        json!({
            "action": "find_entries",
            "root": logs_path.clone(),
            "pattern": "clawd",
            "target_kind": "file",
        }),
    );
    assert_eq!(
        find_args.get("root").and_then(Value::as_str),
        Some(logs_path.as_str())
    );
    assert_eq!(
        find_args.get("pattern").and_then(Value::as_str),
        Some("clawd")
    );
    route.route_reason = "capability_ref=filesystem.read_text_range".to_string();
    let read_args = assert_planner_supplied_tool_call_preserved(
        &state,
        &route,
        &LoopState::new(1),
        "read the selected log tail and synthesize status",
        Some(&logs_path),
        Some(&route.route_reason),
        "fs_basic",
        "read_text_range",
        json!({
            "action": "read_text_range",
            "path": log_path.clone(),
            "mode": "tail",
            "n": 20,
        }),
    );
    assert_eq!(
        read_args.get("path").and_then(Value::as_str),
        Some(log_path.as_str())
    );
    assert_eq!(read_args.get("mode").and_then(Value::as_str), Some("tail"));
    assert_eq!(read_args.get("n").and_then(Value::as_u64), Some(20));
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
    let mut state = test_state_with_enabled_skills(&["fs_basic"]);
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::act_with_chat_finalizer(),
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
    route.route_reason = "capability_ref=filesystem.read_text_range".to_string();

    let read_args = assert_planner_supplied_tool_call_preserved(
        &state,
        &route,
        &LoopState::new(1),
        "read clawd log tail",
        Some(&logs_path),
        Some(&route.route_reason),
        "fs_basic",
        "read_text_range",
        json!({
            "action": "read_text_range",
            "path": log_path.clone(),
            "mode": "tail",
            "n": 20,
        }),
    );
    assert_eq!(
        read_args.get("path").and_then(Value::as_str),
        Some(log_path.as_str())
    );
    assert_eq!(read_args.get("mode").and_then(Value::as_str), Some("tail"));
    assert_eq!(read_args.get("n").and_then(Value::as_u64), Some(20));
}

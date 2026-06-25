use super::*;

#[test]
fn log_content_summary_allows_bounded_slice_with_listing_evidence() {
    let state = test_state_with_registry();
    let loop_state = LoopState::new(2);
    let temp = TempDirGuard::new("log_content_summary_bounded_slice_with_listing");
    let logs_dir = temp.path.join("logs");
    fs::create_dir_all(&logs_dir).expect("mkdir logs");
    let log_path = logs_dir.join("clawd.run.log");
    fs::write(&log_path, "INFO boot\nINFO ready\n").expect("write fixture log");
    let log_path = log_path.display().to_string();
    let logs_path = logs_dir.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = log_path.clone();
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "find_entries",
                "path": logs_path,
                "name_contains": "clawd",
                "files_only": true,
                "names_only": true,
            }),
        },
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": log_path,
                "mode": "tail",
                "n": 20,
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["step_1".to_string(), "step_2".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    assert!(super::super::registry_preferred_skill_matches_route(
        &state, &route
    ));
    assert!(
        !super::super::actions_use_ad_hoc_command_without_route_preferred_skill(
            &state, &route, &actions
        )
    );
    assert!(!should_force_actionable_plan_repair(
        &state,
        Some(&route),
        &loop_state,
        &actions
    ));
}

#[test]
fn concrete_log_file_slice_locator_builds_listing_and_read_plan() {
    let temp = TempDirGuard::new("concrete_log_file_slice_locator");
    let logs_dir = temp.path.join("logs");
    fs::create_dir_all(&logs_dir).expect("mkdir logs");
    let log_path = logs_dir.join("clawd.run.log");
    fs::write(&log_path, "INFO boot\nINFO ready\n").expect("write fixture log");
    let log_path = log_path.display().to_string();
    let mut route = route_result(
        crate::AskMode::planner_execute_plain(),
        true,
        OutputResponseShape::OneSentence,
    );
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = log_path.clone();
    route.resolved_intent = "slice_mode=tail slice_n=20".to_string();

    let plan = content_excerpt_summary_directory_log_slice_deterministic_plan_result(
        "observe bounded log slice",
        Some(&route),
        &LoopState::new(1),
        Some(&log_path),
    )
    .expect("concrete log locator should build structured observation plan");

    let find_action = plan.steps[0].to_agent_action().unwrap();
    let find_args = expect_planned_call(&find_action, "fs_basic", "find_entries");
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
}

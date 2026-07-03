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
fn concrete_log_file_slice_contract_allows_listing_and_read_evidence() {
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

    let contract = route.effective_output_contract();
    let find_policy = crate::evidence_policy::action_policy_for_output_contract(
        Some(&contract),
        "fs_basic",
        &json!({
            "action": "find_entries",
            "path": logs_dir.display().to_string(),
            "target_kind": "file",
        }),
    )
    .expect("content excerpt contract should allow listing evidence");
    assert!(find_policy.is_allowed(), "{find_policy:?}");

    let read_policy = crate::evidence_policy::action_policy_for_output_contract(
        Some(&contract),
        "fs_basic",
        &json!({
            "action": "read_text_range",
            "path": log_path,
            "mode": "tail",
            "n": 20,
        }),
    )
    .expect("content excerpt contract should allow bounded read evidence");
    assert!(read_policy.is_allowed(), "{read_policy:?}");
    assert!(read_policy.action_matches_preferred(), "{read_policy:?}");
}

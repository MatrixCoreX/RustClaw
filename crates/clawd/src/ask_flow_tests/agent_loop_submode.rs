fn agent_loop_selected_route_for_gate() -> crate::RouteResult {
    let mut route = chat_route_for_gate();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.resolved_intent = "List workspace file names under docs".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "docs".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;
    route
}

#[test]
fn pure_chat_route_allows_agent_loop_submode_without_execution_contract() {
    let route = chat_route_for_gate();
    assert!(route_allows_agent_loop_pure_chat_submode(&route));
}

#[test]
fn pure_chat_route_submode_rejects_content_evidence_contract() {
    let mut route = chat_route_for_gate();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;
    assert!(!route_allows_agent_loop_pure_chat_submode(&route));
}

#[test]
fn direct_answer_gate_cannot_override_agent_loop_selected_route() {
    let root = TempDirGuard::new("agent_loop_authority_gate");
    let config_dir = root.path.join("configs");
    std::fs::create_dir_all(&config_dir).expect("create config dir");
    std::fs::write(
        config_dir.join("agent_guard.toml"),
        r#"[agent.loop_guard]
semantic_route_authority = "agent_loop_default"
"#,
    )
    .expect("write agent guard");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.path.clone();
    let route = agent_loop_selected_route_for_gate();
    assert_eq!(
        crate::agent_engine::agent_loop_authority_selected_migration_class(&state, &route),
        Some("exact_path_list")
    );
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let gate = gate_out("direct_answer", gate_contract(false, "none", "none"));

    let outcome = apply_direct_answer_gate_outcome(&state, &mut ctx, "list docs", gate);

    assert!(matches!(outcome, DirectAnswerPreflight::PlannerExecute(_)));
    let route = ctx.route_result.expect("route");
    assert!(route.is_execute_gate());
    assert!(route
        .route_reason
        .contains("direct_answer_gate_demoted_for_agent_loop_authority"));
}

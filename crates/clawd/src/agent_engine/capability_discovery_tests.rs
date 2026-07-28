use super::*;

#[test]
fn group_loader_requires_nonempty_machine_tokens_without_an_arbitrary_count_cap() {
    assert_eq!(
        parse_requested_groups(&json!({"groups": []})).unwrap_err(),
        "capability_group_load_count_invalid"
    );
    assert_eq!(
        parse_requested_groups(&json!({"groups": ["crypto", "weather", "kb"]})).unwrap(),
        vec![
            "crypto".to_string(),
            "kb".to_string(),
            "weather".to_string()
        ]
    );
    assert_eq!(
        parse_requested_groups(&json!({"groups": ["weather group"]})).unwrap_err(),
        "capability_group_load_token_invalid"
    );
    assert_eq!(
        parse_requested_groups(&json!({"groups": ["weather", "weather"]})).unwrap(),
        vec!["weather".to_string()]
    );
    assert_eq!(
        parse_requested_groups(&json!({"groups": ["weather", "crypto"]})).unwrap(),
        vec!["crypto".to_string(), "weather".to_string()]
    );
}

#[test]
fn group_loader_expands_only_exact_registry_groups() {
    let state = crate::AppState::test_default_with_fixture_provider()
        .with_prompt_layers_installed()
        .with_real_skill_registry();
    let task = crate::ClaimedTask {
        claim_attempt: 0,
        task_id: "capability-loader".to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let mut loop_state = LoopState::new();
    loop_state.round_no = 1;
    let loadable = crate::capability_map::planner_loadable_capability_group_names_for_task(
        &state,
        &task,
        &loop_state.loaded_capability_skills,
    );
    let group = loadable
        .first()
        .cloned()
        .expect("fixture must expose an on-demand registry group");
    let mut executed = 0;
    let decision = handle_capability_group_load(
        &state,
        &task,
        &mut loop_state,
        &json!({"groups": [&group]}),
        &format!("load:{group}"),
        1,
        1,
        &mut executed,
    )
    .unwrap();
    assert!(matches!(decision, ActionLoopDecision::StopRound(_)));
    assert_eq!(executed, 1);
    assert!(loop_state.loaded_capability_skills.contains(&group));
    assert!(loop_state
        .last_output
        .as_deref()
        .is_some_and(|output| output.contains(&format!("\"loaded_groups\":[\"{group}\"]"))));

    let error = match handle_capability_group_load(
        &state,
        &task,
        &mut loop_state,
        &json!({"groups": ["not_registered"]}),
        "load:invalid",
        2,
        2,
        &mut executed,
    ) {
        Ok(_) => panic!("unknown registry group must be rejected"),
        Err(error) => error,
    };
    assert!(error.contains("capability_group_not_loadable"));
}

#[test]
fn catalog_tool_searches_then_expands_exact_authorized_contracts() {
    let state = crate::AppState::test_default_with_fixture_provider()
        .with_prompt_layers_installed()
        .with_real_skill_registry();
    let task = crate::ClaimedTask {
        claim_attempt: 0,
        task_id: "capability-search-expand".to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let entry = super::super::capability_catalog::catalog_entries_for_task(&state, &task)
        .into_iter()
        .next()
        .expect("authorized fixture capability");
    let mut loop_state = LoopState::new();
    let mut executed = 0;
    let search = handle_capability_group_load(
        &state,
        &task,
        &mut loop_state,
        &json!({"op": "search", "query": entry.capability_id}),
        "catalog:search",
        1,
        1,
        &mut executed,
    )
    .expect("catalog search");
    assert!(
        matches!(search, ActionLoopDecision::StopRound(signal) if signal == "capability_catalog_searched")
    );
    assert!(loop_state
        .last_output
        .as_deref()
        .is_some_and(|output| output.contains(&entry.contract_sha256)));

    let expand = handle_capability_group_load(
        &state,
        &task,
        &mut loop_state,
        &json!({"op": "expand", "capability_refs": [entry.capability_ref]}),
        "catalog:expand",
        2,
        2,
        &mut executed,
    )
    .expect("catalog expansion");
    assert!(
        matches!(expand, ActionLoopDecision::StopRound(signal) if signal == "capability_contracts_expanded")
    );
    assert!(loop_state
        .loaded_capability_skills
        .contains(&entry.skill_id));
    assert_eq!(executed, 2);
}

#[test]
fn active_scope_set_keeps_every_selected_group_queryable() {
    let mut loop_state = LoopState::new();
    for group in ["alpha", "beta", "gamma", "delta", "epsilon"] {
        activate_registry_groups(&mut loop_state, &[group.to_string()]);
    }

    assert_eq!(loop_state.active_capability_scopes.len(), 5);
    assert_eq!(
        loop_state.loaded_capability_skills,
        BTreeSet::from([
            "alpha".to_string(),
            "beta".to_string(),
            "delta".to_string(),
            "epsilon".to_string(),
            "gamma".to_string(),
        ])
    );

    activate_registry_groups(&mut loop_state, &["beta".to_string()]);
    activate_registry_groups(&mut loop_state, &["zeta".to_string()]);
    assert_eq!(loop_state.active_capability_scopes.len(), 6);
    assert!(loop_state.loaded_capability_skills.contains("beta"));
    assert!(loop_state.loaded_capability_skills.contains("gamma"));
    assert_eq!(
        loop_state
            .active_capability_scopes
            .last()
            .map(String::as_str),
        Some("registry.zeta")
    );
}

#[test]
fn capability_loader_is_a_verified_observe_only_runtime_action() {
    let state = crate::AppState::test_default_with_fixture_provider()
        .with_prompt_layers_installed()
        .with_real_skill_registry();
    let task = crate::ClaimedTask {
        claim_attempt: 0,
        task_id: "verify-capability-loader".to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let plan = crate::PlanResult {
        goal: "load selected machine scope".to_string(),
        missing_slots: Vec::new(),
        needs_confirmation: false,
        output_contract: None,
        steps: vec![crate::plan_step_from_agent_action(
            &crate::AgentAction::CallTool {
                tool: RUNTIME_CAPABILITY_LOADER_TOOL.to_string(),
                args: json!({"groups": ["crypto"]}),
            },
            "step_1".to_string(),
            Vec::new(),
            "runtime scope update".to_string(),
        )],
        planner_notes: String::new(),
        plan_kind: crate::PlanKind::Native,
        raw_plan_text: "{}".to_string(),
    };

    let verified = crate::verifier::verify_plan(
        &state,
        &task,
        crate::verifier::VerifyInput {
            output_contract: None,
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan,
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        crate::verifier::VerifyMode::Enforce,
    );

    assert!(verified.approved, "issues={:?}", verified.issues);
    assert!(!verified.needs_confirmation);
    assert_eq!(
        verified.permission_decision["steps"][0]["action_effect"]["observes"],
        true
    );
}

#[tokio::test]
async fn mcp_catalog_results_activate_exact_permission_scoped_capabilities() {
    let runtime = crate::mcp_runtime::test_support::started_fixture_runtime().await;
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.core.mcp_runtime = std::sync::Arc::clone(&runtime);
    let task = crate::ClaimedTask {
        claim_attempt: 0,
        task_id: "mcp-capability-loader".to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let search = runtime
        .call(
            crate::mcp_runtime::MCP_CATALOG_SEARCH_CAPABILITY,
            json!({"query": "lookup", "limit": 1}),
            None,
        )
        .await
        .expect("catalog search");
    let mut loop_state = LoopState::new();
    let loaded = activate_mcp_search_results(
        &state,
        &task,
        &mut loop_state,
        Some(&json!({
            "mcp_result": serde_json::to_value(search).expect("search result json")
        })),
    );

    assert_eq!(loaded, vec!["mcp.fixture.lookup".to_string()]);
    assert!(loop_state
        .loaded_mcp_capabilities
        .contains("mcp.fixture.lookup"));
    let planner_tools = crate::capability_map::planner_mcp_tools_for_task(
        &state,
        &task,
        &loop_state.loaded_mcp_capabilities,
    );
    let lookup = planner_tools
        .iter()
        .find(|tool| tool.capability == "mcp.fixture.lookup")
        .expect("loaded MCP descriptor");
    assert_eq!(lookup.policy.effect, "observe");
    assert_eq!(lookup.policy.risk_level, "low");
    assert_eq!(lookup.required_args, vec!["query"]);
    runtime.stop().await;
}

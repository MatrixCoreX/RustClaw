#[test]
fn direct_answer_gate_does_not_skip_current_workspace_identity_draft() {
    let root = TempDirGuard::new("gate_workspace_identity_draft");
    let workspace = root.path.join("rustclaw");
    std::fs::create_dir_all(&workspace).expect("workspace dir");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = workspace.clone();
    state.skill_rt.default_locator_search_dir = workspace;
    let mut route = chat_route_for_gate();
    route.resolved_intent =
        "撰写关于 RustClaw 的长文\nanswer_candidate: ## RustClaw\n一段未验证的写作草稿。"
            .to_string();

    assert!(!direct_answer_gate_can_skip_for_pure_chat_draft(
        &state,
        "帮我写一篇关于 RustClaw 的长文",
        Some(&route)
    ));
}

#[test]
fn direct_answer_gate_keeps_locator_draft_under_gate() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let mut route = chat_route_for_gate();
    route.resolved_intent =
        "撰写关于配置文件的长文\nanswer_candidate: ## 配置文件\n一段未验证的写作草稿。".to_string();

    assert!(!direct_answer_gate_can_skip_for_pure_chat_draft(
        &state,
        "帮我写一篇关于 configs/config.toml 的长文",
        Some(&route)
    ));
}

#[test]
fn direct_answer_gate_accepts_distinctive_candidate_bound_in_memory_context() {
    let mut route = chat_route_for_gate();
    route.resolved_intent = "recall_scalar\nanswer_candidate: RC-CONT-CN-0428-A".to_string();
    let gate = gate_out("direct_answer", gate_contract(false, "none", "none"));
    let state = crate::AppState::test_default_with_fixture_provider();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        memory_context_for_execution: Some(
            "### MEMORY_CONTEXT (NOT CURRENT REQUEST)\n\
#### RELEVANT_FACTS\n\
- 当前连续测试的编号为 RC-CONT-CN-0428-A，助手应记住并在后续任务中引用。"
                .to_string(),
        ),
        ..Default::default()
    };

    let outcome = apply_direct_answer_gate_outcome(
        &state,
        &mut ctx,
        "刚才让你记住的连续测试编号是什么？只回答编号。",
        gate,
    );

    assert!(matches!(outcome, DirectAnswerPreflight::DirectAnswer));
    let route = ctx.route_result.expect("route");
    assert!(!route.needs_clarify);
    assert!(!route
        .route_reason
        .contains("direct_answer_gate_unbound_deictic_clarify"));
}

#[test]
fn direct_answer_gate_keeps_observed_result_interpretation_direct() {
    let mut route = chat_route_for_gate();
    route.resolved_intent =
        "Judge whether the previously observed log entries contain abnormal patterns".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        cross_turn_recent_execution_context: Some(
            "### RECENT_EXECUTION_ANCHOR\n\
- latest_request=tail that log 10 lines\n\
- latest_result=2026-04-01 WARN cache miss ratio above baseline | 2026-04-01 ERROR provider timeout while fetching external metadata\n\
### RECENT_EXECUTION_EVENTS\n\
- kind=ask request=tail that log 10 lines result=2026-04-01 ERROR provider timeout while fetching external metadata"
                .to_string(),
        ),
        ..Default::default()
    };
    let mut gate = gate_out(
        "planner_execute",
        gate_contract(true, "none", "content_excerpt_summary"),
    );
    gate.reference_resolution.target = "current_action_result".to_string();
    gate.resolved_user_intent =
        "Judge whether the already observed log entries contain anything abnormal.".to_string();
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome = apply_direct_answer_gate_outcome(
        &state,
        &mut ctx,
        "in one sentence tell me if anything looks abnormal",
        gate,
    );

    assert!(matches!(outcome, DirectAnswerPreflight::DirectAnswer));
    let route = ctx.route_result.expect("route");
    assert!(route.is_chat_gate());
    assert!(route
        .route_reason
        .contains("direct_answer_gate_existing_observed_result_ignored"));
}

#[test]
fn direct_answer_gate_keeps_observed_failure_explanation_direct() {
    let mut route = chat_route_for_gate();
    route.resolved_intent =
        "如果文件不存在，则简短说明原因\nanswer_candidate: 文件不存在，路径可能错误或文件已删除。"
            .to_string();
    route.output_contract.requires_content_evidence = true;
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        cross_turn_recent_execution_context: Some(
            "### RECENT_EXECUTION_ANCHOR\n\
- latest_request=把那个文件发给我\n\
- latest_result=未找到文件：/tmp/not_exists.md，所以无法发送。请确认完整路径或上传该文件\n\
### RECENT_EXECUTION_EVENTS\n\
- kind=ask request=把那个文件发给我 result=未找到文件：/tmp/not_exists.md，所以无法发送。"
                .to_string(),
        ),
        ..Default::default()
    };
    let mut gate = gate_out("direct_answer", gate_contract(true, "none", "none"));
    gate.reference_resolution.target = "unresolved_prior_object".to_string();
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome =
        apply_direct_answer_gate_outcome(&state, &mut ctx, "如果不存在就简短说明原因", gate);

    assert!(matches!(outcome, DirectAnswerPreflight::DirectAnswer));
    let route = ctx.route_result.expect("route");
    assert!(route.is_chat_gate());
    assert!(route
        .route_reason
        .contains("direct_answer_gate_existing_observed_result_ignored"));
}

#[test]
fn direct_answer_gate_still_promotes_locatorless_runtime_observation() {
    let route = chat_route_for_gate();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        cross_turn_recent_execution_context: Some(
            "### RECENT_EXECUTION_ANCHOR\n- latest_request=list logs\n- latest_result=app.log"
                .to_string(),
        ),
        ..Default::default()
    };
    let mut gate = gate_out("planner_execute", gate_contract(true, "none", "none"));
    gate.reference_resolution.target = "none".to_string();
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome = apply_direct_answer_gate_outcome(&state, &mut ctx, "what is the hostname?", gate);

    assert!(matches!(outcome, DirectAnswerPreflight::PlannerExecute(_)));
}

#[test]
fn active_ordered_entries_count_returns_scalar_count() {
    let mut route = chat_route_for_gate();
    route.needs_clarify = true;
    route.set_first_layer_decision(crate::FirstLayerDecision::Clarify);
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
    route.output_contract.requires_content_evidence = true;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        context_bundle_summary: Some(
            "### ACTIVE_EXECUTION_ANCHOR\n\
followup_bound_target: /tmp/docs\n\
followup_ordered_entries: 1:archive | 2:release_checklist.md | 3:service_notes.md\n\
observed_ordered_entries: 1:archive | 2:release_checklist.md | 3:service_notes.md"
                .to_string(),
        ),
        ..Default::default()
    };

    assert_eq!(
        active_ordered_entries_count_direct_answer_candidate(
            "然后告诉我一共有多少个直接子项，只输出数字",
            Some(&ctx),
        ),
        Some("3".to_string())
    );
}

#[test]
fn active_anchor_observed_judgment_promotes_execute_to_chat() {
    let mut route = chat_route_for_gate();
    route.set_first_layer_decision(crate::FirstLayerDecision::PlannerExecute);
    route.route_reason = "structured_anchor_direct_answer_requires_evidence".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        context_bundle_summary: Some(
            "### ACTIVE_EXECUTION_ANCHOR\n\
followup_bound_target: /tmp/test_contract.sqlite\n\
followup_ordered_entries: 1:orders | 2:service_logs | 3:users\n\
observed_bound_target: /tmp/test_contract.sqlite\n\
observed_ordered_entries: 1:orders | 2:service_logs | 3:users"
                .to_string(),
        ),
        ..Default::default()
    };

    assert!(promote_active_anchor_observed_judgment_to_chat(
        "pick the most business-like table",
        Some(&mut ctx),
    ));
    let route = ctx.route_result.as_ref().expect("route");
    assert!(route.is_chat_gate());
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(route.output_contract.requires_content_evidence);
    assert!(route
        .route_reason
        .contains("active_anchor_observed_judgment_to_chat"));
}

#[test]
fn active_anchor_observed_judgment_chat_context_includes_anchor_evidence() {
    let mut route = chat_route_for_gate();
    route.set_first_layer_decision(crate::FirstLayerDecision::DirectAnswer);
    route.route_reason = "active_anchor_observed_judgment_to_chat".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        context_bundle_summary: Some(
            "### ACTIVE_EXECUTION_ANCHOR\n\
followup_bound_target: /tmp/test_contract.sqlite\n\
observed_ordered_entries: 1:orders | 2:service_logs | 3:users\n\
### OTHER_CONTEXT\nignored"
                .to_string(),
        ),
        ..Default::default()
    };

    let rendered = chat_prompt_context_with_route_resolution("<none>", Some(&ctx));

    assert!(rendered.contains("active_execution_anchor_evidence:"));
    assert!(rendered.contains("observed_ordered_entries: 1:orders | 2:service_logs | 3:users"));
    assert!(!rendered.contains("ignored"));
}

#[test]
fn direct_answer_gate_clarifies_locatorless_target_specific_planner_request() {
    let route = chat_route_for_gate();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut gate = gate_out("planner_execute", gate_contract(true, "none", "none"));
    gate.resolved_user_intent =
        "Find the SQLite database in the current project and query the schema version value."
            .to_string();
    gate.reference_resolution.target = "missing_locator".to_string();
    let state = crate::AppState::test_default_with_fixture_provider();
    let contract = output_contract_from_direct_answer_gate(
        gate.output_contract.clone(),
        &crate::IntentOutputContract::default(),
    );

    assert!(direct_answer_gate_planner_needs_unbound_locator_clarify(
        &state,
        "check the schema version of that sqlite database",
        &contract,
        &gate.reference_resolution,
        None,
        false,
    ));

    let outcome = apply_direct_answer_gate_outcome(
        &state,
        &mut ctx,
        "check the schema version of that sqlite database",
        gate,
    );

    assert!(matches!(outcome, DirectAnswerPreflight::Clarify(_)));
    let route = ctx.route_result.expect("route");
    assert!(route.needs_clarify);
    assert!(route
        .route_reason
        .contains("direct_answer_gate_unbound_deictic_clarify"));
}

#[test]
fn direct_answer_gate_allows_locatorless_targetless_planner_request() {
    let route = chat_route_for_gate();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let gate = gate_out("planner_execute", gate_contract(true, "none", "none"));
    let state = crate::AppState::test_default_with_fixture_provider();
    let contract = output_contract_from_direct_answer_gate(
        gate.output_contract.clone(),
        &crate::IntentOutputContract::default(),
    );

    assert!(!direct_answer_gate_planner_needs_unbound_locator_clarify(
        &state,
        "detect the current runtime package manager",
        &contract,
        &gate.reference_resolution,
        None,
        false,
    ));

    let outcome = apply_direct_answer_gate_outcome(
        &state,
        &mut ctx,
        "detect the current runtime package manager",
        gate,
    );

    assert!(matches!(outcome, DirectAnswerPreflight::PlannerExecute(_)));
    let route = ctx.route_result.expect("route");
    assert!(route.is_execute_gate());
    assert!(!route.needs_clarify);
    assert!(!route
        .route_reason
        .contains("direct_answer_gate_unbound_deictic_clarify"));
}

#[test]
fn direct_answer_gate_clarifies_unbound_path_candidate_for_delivery_and_preserves_contract() {
    let mut route = chat_route_for_gate();
    route.resolved_intent = concat!(
        "Deliver the requested local config file without pasting its body\n",
        "answer_candidate: /tmp/untrusted/config.toml"
    )
    .to_string();
    let mut contract = gate_contract(true, "none", "none");
    contract.response_shape = "file_token".to_string();
    contract.delivery_required = true;
    contract.delivery_intent = "file_single".to_string();
    let gate = gate_out("planner_execute", contract);
    let state = crate::AppState::test_default_with_fixture_provider();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let outcome = apply_direct_answer_gate_outcome(
        &state,
        &mut ctx,
        "send me the local config file without pasting the body",
        gate,
    );

    assert!(matches!(outcome, DirectAnswerPreflight::Clarify(_)));
    let route = ctx.route_result.expect("route");
    assert!(route.needs_clarify);
    assert!(route.clarify_question.is_empty());
    assert!(route.wants_file_delivery);
    assert!(route.output_contract.delivery_required);
    assert_eq!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::FileToken
    );
    assert_eq!(
        route.output_contract.delivery_intent,
        crate::OutputDeliveryIntent::FileSingle
    );
}

#[test]
fn direct_answer_gate_allows_locatorless_scalar_runtime_execution() {
    let mut route = chat_route_for_gate();
    route.resolved_intent =
        "runtime_scalar\nanswer_candidate: not-current-runtime-user-000".to_string();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut contract = gate_contract(true, "none", "none");
    contract.response_shape = "scalar".to_string();
    let mut gate = gate_out("planner_execute", contract);
    gate.resolved_user_intent = "Report the current runtime account name.".to_string();
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome =
        apply_direct_answer_gate_outcome(&state, &mut ctx, "current runtime account", gate);

    assert!(matches!(outcome, DirectAnswerPreflight::PlannerExecute(_)));
    let route = ctx.route_result.expect("route");
    assert!(route.is_execute_gate());
    assert!(!route.needs_clarify);
    assert_eq!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Scalar
    );
    assert!(!route
        .route_reason
        .contains("direct_answer_gate_unbound_deictic_clarify"));
}

#[test]
fn direct_answer_gate_keeps_verified_runtime_identity_scalar_direct() {
    let Some(runtime_user) = ["USER", "LOGNAME", "USERNAME"]
        .into_iter()
        .filter_map(|key| std::env::var(key).ok())
        .map(|value| value.trim().to_string())
        .find(|value| !value.is_empty())
    else {
        return;
    };
    let mut route = chat_route_for_gate();
    route.resolved_intent = format!("runtime_scalar\nanswer_candidate: {runtime_user}");
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut contract = gate_contract(true, "none", "none");
    contract.response_shape = "scalar".to_string();
    let mut gate = gate_out("planner_execute", contract);
    gate.resolved_user_intent = "Report the current runtime account name.".to_string();
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome =
        apply_direct_answer_gate_outcome(&state, &mut ctx, "current runtime account", gate);

    assert!(matches!(outcome, DirectAnswerPreflight::DirectAnswer));
    let route = ctx.route_result.expect("route");
    assert!(!route.needs_clarify);
    assert!(!route.is_execute_gate());
    assert!(route
        .route_reason
        .contains("direct_answer_gate_bound_candidate_evidence"));
}

#[test]
fn direct_answer_gate_clarifies_unbound_existing_file_delivery_without_locator() {
    let mut route = chat_route_for_gate();
    route.ask_mode = crate::AskMode::planner_execute_plain();
    route.resolved_intent =
        "Deliver the local configuration file without pasting content.".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = true;
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut contract = gate_contract(true, "current_workspace", "none");
    contract.delivery_required = true;
    contract.response_shape = "file_token".to_string();
    contract.delivery_intent = "file_single".to_string();
    let gate = gate_out("planner_execute", contract);
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome = apply_direct_answer_gate_outcome(
        &state,
        &mut ctx,
        "把那份本地配置直接甩给我，别贴正文",
        gate,
    );

    assert!(matches!(outcome, DirectAnswerPreflight::Clarify(_)));
    let route = ctx.route_result.expect("route");
    assert!(route.needs_clarify);
    assert!(route.clarify_question.is_empty());
    assert!(route.wants_file_delivery);
    assert!(route.output_contract.delivery_required);
    assert_eq!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::FileToken
    );
    assert_eq!(
        route.output_contract.delivery_intent,
        crate::OutputDeliveryIntent::FileSingle
    );
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(route.output_contract.locator_hint.is_empty());
}

#[test]
fn direct_answer_gate_allows_generated_file_delivery_without_locator() {
    let route = chat_route_for_gate();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut contract = gate_contract(true, "current_workspace", "generated_file_delivery");
    contract.delivery_required = true;
    contract.response_shape = "file_token".to_string();
    contract.delivery_intent = "file_single".to_string();
    let gate = gate_out("planner_execute", contract);
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome = apply_direct_answer_gate_outcome(
        &state,
        &mut ctx,
        "写一份部署清单，保存成 md 文件发给我",
        gate,
    );

    assert!(matches!(outcome, DirectAnswerPreflight::PlannerExecute(_)));
    let route = ctx.route_result.expect("route");
    assert!(route.is_execute_gate());
    assert!(route.output_contract.delivery_required);
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::GeneratedFileDelivery
    );
}

#[test]
fn direct_answer_gate_allows_locatorless_workspace_project_summary_semantic() {
    let gate = gate_out(
        "planner_execute",
        gate_contract(true, "none", "workspace_project_summary"),
    );
    let state = crate::AppState::test_default_with_fixture_provider();
    let contract = output_contract_from_direct_answer_gate(
        gate.output_contract.clone(),
        &crate::IntentOutputContract::default(),
    );

    assert!(!direct_answer_gate_planner_needs_unbound_locator_clarify(
        &state,
        "summarize this project",
        &contract,
        &gate.reference_resolution,
        None,
        false,
    ));
}

#[test]
fn direct_answer_gate_promotes_artifact_listing_candidate_to_planner() {
    let mut route = chat_route_for_gate();
    route.resolved_intent = concat!(
            "List the first five entries under the selected workspace directory\n",
            "answer_candidate: act_plan.log, clawd.log, clawd.run.log, clawd.test.log, clawd_manual.log"
        )
        .to_string();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let gate = gate_out("direct_answer", gate_contract(false, "none", "none"));
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome =
        apply_direct_answer_gate_outcome(&state, &mut ctx, "list the selected logs", gate);

    assert!(matches!(outcome, DirectAnswerPreflight::PlannerExecute(_)));
    let route = ctx.route_result.expect("route");
    assert_eq!(route.ask_mode, crate::AskMode::planner_execute_plain());
    assert!(route.is_execute_gate());
    assert!(route.output_contract.requires_content_evidence);
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::FileNames
    );
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::CurrentWorkspace
    );
    assert!(route
        .route_reason
        .contains("direct_answer_gate_artifact_listing_execute"));
}

#[test]
fn direct_answer_gate_does_not_promote_non_artifact_example_list() {
    let mut route = chat_route_for_gate();
    route.resolved_intent =
        "Give simple examples\nanswer_candidate: apple, banana, cherry".to_string();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let gate = gate_out("direct_answer", gate_contract(false, "none", "none"));
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome = apply_direct_answer_gate_outcome(&state, &mut ctx, "give examples", gate);

    assert!(matches!(outcome, DirectAnswerPreflight::DirectAnswer));
    let route = ctx.route_result.expect("route");
    assert!(route.is_chat_gate());
    assert!(!route.output_contract.requires_content_evidence);
}

#[test]
fn direct_answer_gate_promotes_inline_json_transform_to_planner() {
    let mut route = chat_route_for_gate();
    route.resolved_intent =
        "Apply the provided structured transform payload\nanswer_candidate: beta, alpha"
            .to_string();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let gate = gate_out("direct_answer", gate_contract(false, "none", "none"));
    let state = crate::AppState::test_default_with_fixture_provider();
    let request = r#"{"action":"transform_data","data":[{"name":"alpha","score":7},{"name":"beta","score":12}],"ops":[{"op":"sort","by":"score","order":"desc"},{"op":"project","fields":["name"]}]}"#;

    let outcome = apply_direct_answer_gate_outcome(&state, &mut ctx, request, gate);

    assert!(matches!(outcome, DirectAnswerPreflight::PlannerExecute(_)));
    let route = ctx.route_result.expect("route");
    assert!(route.is_execute_gate());
    assert!(route.output_contract.requires_content_evidence);
    assert!(route
        .route_reason
        .contains("direct_answer_gate_inline_transform_execute"));
}

#[test]
fn direct_answer_gate_promotes_inline_json_table_candidate_to_transform_planner() {
    let mut route = chat_route_for_gate();
    route.resolved_intent = "Sort provided JSON array by score descending and output as markdown table.\nanswer_candidate: | name | score |\n|------|-------|\n| beta | 12 |\n| gamma | 9 |\n| alpha | 7 |".to_string();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut contract = gate_contract(true, "none", "none");
    contract.response_shape = "strict".to_string();
    let gate = gate_out("planner_execute", contract);
    let state = crate::AppState::test_default_with_fixture_provider();
    let request = r#"把这个 JSON 数组按 score 从高到低排序，再输出成 markdown 表格：[{"name":"alpha","score":7},{"name":"beta","score":12},{"name":"gamma","score":9}]"#;

    let outcome = apply_direct_answer_gate_outcome(&state, &mut ctx, request, gate);

    assert!(matches!(outcome, DirectAnswerPreflight::PlannerExecute(_)));
    let route = ctx.route_result.expect("route");
    assert!(route.is_execute_gate());
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(route
        .route_reason
        .contains("direct_answer_gate_inline_transform_execute"));
}

#[test]
fn direct_answer_gate_promotes_fenced_inline_json_table_candidate_to_transform_planner() {
    let mut route = chat_route_for_gate();
    route.resolved_intent = "Sort provided JSON array by score descending and output as markdown table.\nanswer_candidate: ```markdown\n| name | score |\n|------|-------|\n| beta | 12 |\n| gamma | 9 |\n| alpha | 7 |\n```".to_string();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut contract = gate_contract(true, "none", "none");
    contract.response_shape = "strict".to_string();
    let gate = gate_out("planner_execute", contract);
    let state = crate::AppState::test_default_with_fixture_provider();
    let request = r#"把这个 JSON 数组按 score 从高到低排序，再输出成 markdown 表格：[{"name":"alpha","score":7},{"name":"beta","score":12},{"name":"gamma","score":9}]"#;

    let outcome = apply_direct_answer_gate_outcome(&state, &mut ctx, request, gate);

    assert!(matches!(outcome, DirectAnswerPreflight::PlannerExecute(_)));
    let route = ctx.route_result.expect("route");
    assert!(route.is_execute_gate());
    assert!(route
        .route_reason
        .contains("direct_answer_gate_inline_transform_execute"));
}

#[test]
fn direct_answer_gate_promotes_inline_json_planner_without_candidate_to_transform_planner() {
    let mut route = chat_route_for_gate();
    route.resolved_intent =
        "Sort provided JSON array by score descending and output as markdown table.".to_string();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut contract = gate_contract(false, "none", "none");
    contract.response_shape = "strict".to_string();
    let gate = gate_out("planner_execute", contract);
    let state = crate::AppState::test_default_with_fixture_provider();
    let request = r#"把这个 JSON 数组按 score 从高到低排序，再输出成 markdown 表格：[{"name":"alpha","score":7},{"name":"beta","score":12},{"name":"gamma","score":9}]"#;

    let outcome = apply_direct_answer_gate_outcome(&state, &mut ctx, request, gate);

    assert!(matches!(outcome, DirectAnswerPreflight::PlannerExecute(_)));
    let route = ctx.route_result.expect("route");
    assert!(route.is_execute_gate());
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(route.output_contract.requires_content_evidence);
    assert!(route
        .route_reason
        .contains("direct_answer_gate_inline_transform_execute"));
}

#[test]
fn direct_answer_gate_marks_contextual_inline_payload_execution() {
    let route = chat_route_for_gate();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut contract = gate_contract(true, "none", "none");
    contract.response_shape = "strict".to_string();
    let gate = gate_out("planner_execute", contract);
    let state = crate::AppState::test_default_with_fixture_provider();
    let request = r#"[{"name":"alpha","score":7},{"name":"beta","score":12}]"#;

    let outcome = apply_direct_answer_gate_outcome(&state, &mut ctx, request, gate);

    assert!(matches!(outcome, DirectAnswerPreflight::PlannerExecute(_)));
    let route = ctx.route_result.expect("route");
    assert!(route.is_execute_gate());
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(
        route
            .route_reason
            .contains("inline_structured_payload_context_execute"),
        "route_reason={:?}, output_contract={:?}",
        route.route_reason,
        route.output_contract
    );
}

#[test]
fn inline_json_transform_context_promotion_uses_strict_execution_contract() {
    let mut route = chat_route_for_gate();
    route.route_reason = "executionless_route_downgraded_to_direct_answer".to_string();
    route.resolved_intent =
        "Transform inline JSON.\nanswer_candidate: [{\"city\":\"Tokyo\"},{\"city\":\"Osaka\"}]"
            .to_string();
    let request = r#"{"action":"transform_data","data":[{"city":"Tokyo","temp":22},{"city":"Osaka","temp":24}],"ops":[{"op":"project","fields":["city"]}]}"#;
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert!(promote_inline_json_transform_context_to_planner(
        &mut ctx, request
    ));
    let route = ctx.route_result.expect("route");
    assert!(route.is_execute_gate());
    assert_eq!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Strict
    );
    assert!(route.output_contract.requires_content_evidence);
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(route
        .route_reason
        .contains("inline_json_transform_structured_execute"));
    assert_eq!(
        route.resolved_intent,
        format!("{request}\nanswer_candidate: [{{\"city\":\"Tokyo\"}},{{\"city\":\"Osaka\"}}]")
    );
}

#[test]
fn direct_answer_gate_promotes_explicit_readme_summary_to_planner() {
    let root = TempDirGuard::new("gate_bare_readme_summary");
    std::fs::write(root.path.join("README.md"), "# Demo\n\nLocal readme body")
        .expect("write readme");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.path.clone();
    state.skill_rt.default_locator_search_dir = root.path.clone();
    let mut route = chat_route_for_gate();
    route.resolved_intent =
            "Read the README and summarize it in exactly three sentences\nanswer_candidate: synthetic summary"
                .to_string();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut contract = gate_contract(true, "current_workspace", "none");
    contract.locator_hint = "README or README.md".to_string();
    contract.exact_sentence_count = Some(3);
    let gate = gate_out("planner_execute", contract);

    let current_request = "读一下 README.md 然后用恰好三句话总结，不要多也不要少";
    assert!(locator_hint_mentions_current_request(
        "README or README.md",
        current_request
    ));
    assert!(current_request_mentions_resolvable_gate_locator(
        &state,
        current_request,
        &crate::IntentOutputContract {
            requires_content_evidence: true,
            locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
            locator_hint: "README or README.md".to_string(),
            ..crate::IntentOutputContract::default()
        },
    ));

    let outcome = apply_direct_answer_gate_outcome(&state, &mut ctx, current_request, gate);

    assert!(matches!(outcome, DirectAnswerPreflight::PlannerExecute(_)));
    let route = ctx.route_result.expect("route");
    assert!(route.is_execute_gate());
    assert!(route.output_contract.requires_content_evidence);
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Path
    );
    assert_eq!(
        route.output_contract.locator_hint,
        root.path.join("README.md").display().to_string()
    );
    assert!(route.route_reason.contains("direct_answer_gate_"));
    assert!(route.route_reason.contains("_execute"));
}

#[test]
fn direct_answer_gate_promotes_package_manager_detect_to_planner() {
    let mut route = chat_route_for_gate();
    route.resolved_intent = "package manager detection\nanswer_candidate: not observed".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::PackageManagerDetection;
    route.output_contract.requires_content_evidence = true;
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let gate = gate_out("direct_answer", gate_contract(false, "none", "none"));
    let state = crate::AppState::test_default_with_fixture_provider();
    let request = "consulta el gestor de paquetes detectado";

    let outcome = apply_direct_answer_gate_outcome(&state, &mut ctx, request, gate);

    assert!(matches!(outcome, DirectAnswerPreflight::PlannerExecute(_)));
    let route = ctx.route_result.expect("route");
    assert!(route.is_execute_gate());
    assert!(route.output_contract.requires_content_evidence);
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::PackageManagerDetection
    );
    assert!(route
        .route_reason
        .contains("direct_answer_gate_package_manager_detect_execute"));
}

#[test]
fn direct_answer_gate_promotes_package_install_preview_without_locator() {
    let mut route = chat_route_for_gate();
    route.route_reason =
            "llm_semantic_contract_repair:dry_run_command_discovery_requires_local_observation; executionless_route_downgraded_to_direct_answer"
                .to_string();
    route.resolved_intent =
        "package preview\nanswer_candidate: command: sudo -n apt-get install -y ripgrep"
            .to_string();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut gate = gate_out("planner_execute", gate_contract(true, "none", "none"));
    gate.resolved_user_intent =
        "Show the package install dry-run preview without installing.".to_string();
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome = apply_direct_answer_gate_outcome(
        &state,
        &mut ctx,
        "ripgrep 설치는 하지 말고 dry-run 으로 어떤 명령이 될지만 알려줘.",
        gate,
    );

    assert!(matches!(outcome, DirectAnswerPreflight::PlannerExecute(_)));
    let route = ctx.route_result.expect("route");
    assert!(route.is_execute_gate());
    assert!(route.output_contract.requires_content_evidence);
    assert!(route.resolved_intent.contains("answer_candidate:"));
    assert!(!route
        .route_reason
        .contains("direct_answer_gate_executionless_promotion_blocked"));
}

#[test]
fn direct_answer_gate_can_skip_self_contained_inline_json_explanation() {
    let mut route = chat_route_for_gate();
    route.resolved_intent =
        "Explain inline JSON records\nanswer_candidate: two score records".to_string();
    let request =
        r#"解释这个 JSON 代表什么：[{"name":"alpha","score":7},{"name":"beta","score":12}]"#;
    let surface = crate::intent::surface_signals::analyze_prompt_surface(request);

    assert!(
        direct_answer_gate_can_skip_for_self_contained_payload(request, Some(&route),),
        "surface={surface:?}"
    );
}

#[test]
fn direct_answer_gate_keeps_self_contained_inline_json_array_explanation_direct() {
    let mut route = chat_route_for_gate();
    route.resolved_intent = concat!(
        "Explain the meaning and structure of the provided JSON array: ",
        r#"[{"name":"alpha","score":7},{"name":"beta","score":12}]. "#,
        "Preserve the original order as specified."
    )
    .to_string();
    route.route_reason = "The request is for explanation/interpretation of embedded structured data. The user explicitly specifies no sorting. This is a pure discussion task requiring no external retrieval, execution, or workspace inspection.".to_string();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut gate = gate_out("direct_answer", gate_contract(false, "none", "none"));
    gate.resolved_user_intent =
        "Explain the meaning and structure of the provided JSON array.".to_string();
    gate.reason =
        "Self-contained embedded structured data; no external retrieval is needed.".to_string();
    let state = crate::AppState::test_default_with_fixture_provider();
    let request = r#"Explain what this JSON represents without sorting it: [{"name":"alpha","score":7},{"name":"beta","score":12}]"#;

    let outcome = apply_direct_answer_gate_outcome(&state, &mut ctx, request, gate);

    assert!(matches!(outcome, DirectAnswerPreflight::DirectAnswer));
    let route = ctx.route_result.expect("route");
    assert!(!route.needs_clarify);
    assert!(!route
        .route_reason
        .contains("direct_answer_gate_unbound_deictic_clarify"));
}

#[test]
fn direct_answer_gate_does_not_skip_inline_json_transform_payload() {
    let mut route = chat_route_for_gate();
    route.resolved_intent =
        "Apply the provided structured transform payload\nanswer_candidate: beta, alpha"
            .to_string();
    let request = r#"{"action":"transform_data","data":[{"name":"alpha","score":7},{"name":"beta","score":12}],"ops":[{"op":"sort","by":"score","order":"desc"},{"op":"project","fields":["name"]}]}"#;
    let surface = crate::intent::surface_signals::analyze_prompt_surface(request);

    assert!(
        !direct_answer_gate_can_skip_for_self_contained_payload(request, Some(&route),),
        "surface={surface:?}"
    );
}

#[test]
fn direct_answer_gate_skip_rejects_locator_payload() {
    let mut route = chat_route_for_gate();
    route.output_contract.locator_hint = "README.md".to_string();

    assert!(!direct_answer_gate_can_skip_for_self_contained_payload(
        r#"读取 README.md 并按 [{"field":"score"}] 排序"#,
        Some(&route),
    ));
}

#[test]
fn direct_answer_gate_skips_active_text_mutation_without_locator() {
    let mut route = chat_route_for_gate();
    route.route_confidence = Some(0.72);
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskCorrect),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
            should_interrupt_active_run: false,
            state_patch: Some(serde_json::json!({"format": "three-step checklist"})),
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    assert!(direct_answer_gate_can_skip_for_active_task_text_mutation(
        "Actually switch it to a three-step checklist.",
        Some(&ctx)
    ));
}

#[test]
fn direct_answer_gate_skips_active_text_mutation_with_interrupt_flag() {
    let mut route = chat_route_for_gate();
    route.route_confidence = Some(0.72);
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskScopeUpdate),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
            should_interrupt_active_run: true,
            state_patch: Some(serde_json::json!({
                "required_visible_literals": ["80 characters", "body only"]
            })),
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    assert!(direct_answer_gate_can_skip_for_active_task_text_mutation(
        "Make it less technical, under 80 characters, body only.",
        Some(&ctx)
    ));
}

#[test]
fn direct_answer_gate_skips_active_observed_output_chat_repair() {
    let mut route = chat_route_for_gate();
    route.route_confidence = Some(0.72);
    route.route_reason = "active_observed_output_chat_repair".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert!(direct_answer_gate_can_skip_for_active_observed_output_chat_repair(Some(&ctx)));
}

#[test]
fn direct_answer_gate_outcome_preserves_active_text_mutation_from_clarify() {
    let mut route = chat_route_for_gate();
    route.route_confidence = Some(0.72);
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskScopeUpdate),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
            should_interrupt_active_run: true,
            state_patch: Some(serde_json::json!({
                "required_visible_literals": ["80 characters", "body only"]
            })),
            attachment_processing_required: false,
        }),
        ..Default::default()
    };
    let mut gate = gate_out("clarify", gate_contract(false, "none", "none"));
    gate.clarify_question = "Need a topic before rewriting.".to_string();
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome = apply_direct_answer_gate_outcome(
        &state,
        &mut ctx,
        "Make it less technical, under 80 characters, body only.",
        gate,
    );

    assert!(matches!(outcome, DirectAnswerPreflight::DirectAnswer));
    let route = ctx.route_result.expect("route");
    assert!(route
        .route_reason
        .contains("direct_answer_gate_active_task_text_mutation_ignored"));
    assert!(!route.needs_clarify);
}

#[test]
fn chat_route_context_keeps_active_text_mutation_draft_as_semantic_anchor() {
    let mut route = chat_route_for_gate();
    route.resolved_intent = "修正当前方案文档的目标用户描述，将受众从老板改为开发者".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskScopeUpdate),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
            should_interrupt_active_run: false,
            state_patch: None,
            attachment_processing_required: false,
        }),
        semantic_answer_candidate_draft: Some(
            "目标用户：开发者。正文应围绕开发者的使用场景展开。".to_string(),
        ),
        ..Default::default()
    };

    let context = chat_prompt_context_with_route_resolution("<none>", Some(&ctx));

    assert!(context.contains("active_task_semantic_draft:"));
    assert!(context.contains("开发者"));
    assert!(context.contains("Non-evidence writing draft"));
}

#[test]
fn chat_route_context_omits_freeform_route_reason_text() {
    let mut route = chat_route_for_gate();
    route.resolved_intent = "Draft a release note".to_string();
    route.route_reason =
        "MEMORY said the old topic was login; direct_answer_gate_execute:workspace_summary"
            .to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let context = chat_prompt_context_with_route_resolution("<none>", Some(&ctx));

    assert!(context.contains("route_reason_markers:"));
    assert!(context.contains("direct_answer_gate_execute:workspace_summary"));
    assert!(!context.contains("old topic was login"));
    assert!(!context.contains("route_reason: MEMORY"));
}

#[test]
fn chat_route_context_exposes_structured_required_visible_literals() {
    let mut route = chat_route_for_gate();
    route.resolved_intent = "Update the active draft for the corrected audience.".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskCorrect),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
            should_interrupt_active_run: false,
            state_patch: Some(serde_json::json!({
                "required_visible_literals": ["开发者"],
                "forbidden_visible_literals": ["老板"],
                "replacement_pairs": [{"from": "老板", "to": "开发者"}]
            })),
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    let context = chat_prompt_context_with_route_resolution("<none>", Some(&ctx));

    assert!(context.contains("active_task_required_visible_literals: 开发者"));
    assert!(context.contains("active_task_replacement_pairs: 老板 -> 开发者"));
    assert!(context.contains("active_task_forbidden_visible_literals: 老板"));
    assert!(context.contains("must visibly contain"));
}

#[test]
fn chat_route_context_omits_pending_replacement_placeholder() {
    let mut route = chat_route_for_gate();
    route.resolved_intent = "Rewrite the active summary for beginners.".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskScopeUpdate),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
            should_interrupt_active_run: false,
            state_patch: Some(serde_json::json!({
                "replacement_pairs": [{
                    "from": "RustClaw 是一个本地工具，帮助你在电脑上用聊天的方式管理任务。",
                    "to": "[pending_beginner_rephrase]"
                }],
                "required_visible_literals": ["[pending_beginner_rephrase]"]
            })),
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    let context = chat_prompt_context_with_route_resolution("<none>", Some(&ctx));

    assert!(!context.contains("pending_beginner_rephrase"));
    assert!(!context.contains("active_task_replacement_pairs:"));
    assert!(!context.contains("active_task_required_visible_literals:"));
}

#[test]
fn required_visible_literals_accepts_protocol_aliases() {
    let state_patch = serde_json::json!({
        "required_visible_literals": ["开发者", " developer "],
        "visible_constraints": {
            "literals": [{"literal": "`SDK v2`"}]
        }
    });

    assert_eq!(
        required_visible_literals_from_state_patch(&state_patch),
        vec!["开发者", "developer", "SDK v2"]
    );
}

#[test]
fn replacement_pairs_and_forbidden_literals_accept_structured_protocol() {
    let state_patch = serde_json::json!({
        "replacement_pairs": [
            {"from": "老板", "to": "开发者"},
            {"old": "v1", "new": "v2"}
        ],
        "visible_constraints": {
            "forbidden_visible_literals": ["internal only"]
        }
    });

    assert_eq!(
        replacement_pairs_from_state_patch(&state_patch),
        vec![
            super::ActiveTaskReplacementPair {
                from: "老板".to_string(),
                to: "开发者".to_string()
            },
            super::ActiveTaskReplacementPair {
                from: "v1".to_string(),
                to: "v2".to_string()
            }
        ]
    );
    assert_eq!(
        forbidden_visible_literals_from_state_patch(&state_patch),
        vec!["internal only", "老板", "v1"]
    );
}

#[test]
fn replacement_pairs_skip_pending_machine_placeholder_target() {
    let state_patch = serde_json::json!({
        "replacement_pairs": [
            {"from": "old visible text", "to": "[pending_beginner_rephrase]"},
            {"from": "老板", "to": "开发者"}
        ],
        "required_visible_literals": ["[pending_beginner_rephrase]"]
    });

    assert_eq!(
        replacement_pairs_from_state_patch(&state_patch),
        vec![super::ActiveTaskReplacementPair {
            from: "老板".to_string(),
            to: "开发者".to_string()
        }]
    );
    assert_eq!(
        required_visible_literals_from_state_patch(&state_patch),
        vec!["开发者"]
    );
}

#[test]
fn active_task_required_visible_literal_guard_prefixes_missing_literal() {
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(chat_route_for_gate()),
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskScopeUpdate),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
            should_interrupt_active_run: false,
            state_patch: Some(serde_json::json!({
                "replacement_pairs": [{"from": "老板", "to": "开发者"}]
            })),
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    let answer = ensure_active_task_required_visible_literals(
        "系统瓶颈影响交付，目标提升吞吐量。".to_string(),
        Some(&ctx),
    );

    assert!(answer.starts_with("开发者: "));
}

#[test]
fn active_task_required_visible_literal_guard_ignores_untyped_output_constraints() {
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(chat_route_for_gate()),
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskScopeUpdate),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
            should_interrupt_active_run: false,
            state_patch: Some(serde_json::json!({
                "required_visible_literals": ["under 80 characters", "body only"]
            })),
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    let answer = ensure_active_task_required_visible_literals(
        "Invest in this focused plan to reduce risk and improve delivery speed.".to_string(),
        Some(&ctx),
    );

    assert_eq!(
        answer,
        "Invest in this focused plan to reduce risk and improve delivery speed."
    );
}

#[test]
fn active_task_required_content_literal_guard_requires_current_request_source() {
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(chat_route_for_gate()),
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskCorrect),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
            should_interrupt_active_run: false,
            state_patch: Some(serde_json::json!({
                "required_content_literals": [
                    "RustClaw 是一个用 Rust 写的自动化工具",
                    "支持多平台消息渠道统一管理"
                ]
            })),
            attachment_processing_required: false,
        }),
        original_user_request: Some("改成三条短要点，每条不超过 15 个字".to_string()),
        ..Default::default()
    };

    let answer = ensure_active_task_required_visible_literals(
        "本地自动化\n多渠道接入\n新手易配置".to_string(),
        Some(&ctx),
    );

    assert_eq!(answer, "本地自动化\n多渠道接入\n新手易配置");
}

#[test]
fn active_task_required_content_literal_guard_keeps_user_supplied_value() {
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(chat_route_for_gate()),
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskCorrect),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
            should_interrupt_active_run: false,
            state_patch: Some(serde_json::json!({
                "required_content_literals": ["Python 3.11"]
            })),
            attachment_processing_required: false,
        }),
        original_user_request: Some("Correction: it should be Python 3.11, not 3.10".to_string()),
        ..Default::default()
    };

    let answer = ensure_active_task_required_visible_literals(
        "Deploy after checking the runtime.".to_string(),
        Some(&ctx),
    );

    assert!(answer.starts_with("Python 3.11: "));
}

#[test]
fn active_task_required_visible_literal_guard_leaves_existing_literal() {
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(chat_route_for_gate()),
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskCorrect),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
            should_interrupt_active_run: false,
            state_patch: Some(serde_json::json!({
                "required_content_literals": ["developer"]
            })),
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    let answer = ensure_active_task_required_visible_literals(
        "This version is for Developer onboarding.".to_string(),
        Some(&ctx),
    );

    assert_eq!(answer, "This version is for Developer onboarding.");
}

#[test]
fn direct_answer_gate_does_not_skip_active_text_mutation_with_explicit_file_target() {
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(chat_route_for_gate()),
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskCorrect),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
            should_interrupt_active_run: false,
            state_patch: Some(serde_json::json!({"format": "three-step checklist"})),
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    assert!(!direct_answer_gate_can_skip_for_active_task_text_mutation(
        "In README.md, switch it to a three-step checklist.",
        Some(&ctx)
    ));
}

#[test]
fn direct_answer_gate_ignores_background_only_promotion_for_bound_answer_candidate() {
    let mut route = chat_route_for_gate();
    route.resolved_intent =
        "User wants to output only the final checklist.\nanswer_candidate: final_checklist"
            .to_string();
    let promoted_contract = crate::IntentOutputContract {
        requires_content_evidence: true,
        locator_kind: crate::OutputLocatorKind::Path,
        locator_hint: "README.md".to_string(),
        ..crate::IntentOutputContract::default()
    };

    assert!(
        direct_answer_gate_promotion_depends_only_on_background_context(
            &crate::AppState::test_default_with_fixture_provider(),
            "Output only the final checklist.",
            &route,
            &promoted_contract,
            &DirectAnswerGateReferenceResolutionOut::default(),
            false,
        )
    );
}

#[test]
fn direct_answer_gate_ignores_background_only_promotion_without_answer_candidate() {
    let mut route = chat_route_for_gate();
    route.resolved_intent = "Draft a standalone user-facing text artifact.".to_string();
    let promoted_contract = crate::IntentOutputContract {
        requires_content_evidence: true,
        locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
        ..crate::IntentOutputContract::default()
    };

    assert!(
        direct_answer_gate_promotion_depends_only_on_background_context(
            &crate::AppState::test_default_with_fixture_provider(),
            "Draft a standalone user-facing text artifact.",
            &route,
            &promoted_contract,
            &DirectAnswerGateReferenceResolutionOut::default(),
            false,
        )
    );
}

#[test]
fn direct_answer_gate_planner_execute_background_only_promotion_stays_direct() {
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(chat_route_for_gate()),
        ..Default::default()
    };
    let gate = DirectAnswerGateOut {
        decision: "planner_execute".to_string(),
        reason: "context_required".to_string(),
        confidence: 0.92,
        clarify_question: String::new(),
        resolved_user_intent: "Draft a standalone user-facing text artifact.".to_string(),
        reference_resolution: DirectAnswerGateReferenceResolutionOut::default(),
        output_contract: DirectAnswerGateContractOut {
            response_shape: "free".to_string(),
            exact_sentence_count: None,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: "current_workspace".to_string(),
            delivery_intent: "none".to_string(),
            semantic_kind: "none".to_string(),
            locator_hint: String::new(),
            self_extension: DirectAnswerGateSelfExtensionOut::default(),
        },
        state_patch: None,
    };

    let preflight = apply_direct_answer_gate_outcome(
        &crate::AppState::test_default_with_fixture_provider(),
        &mut ctx,
        "Draft a standalone user-facing text artifact.",
        gate,
    );

    assert!(matches!(preflight, DirectAnswerPreflight::DirectAnswer));
    assert!(ctx.route_result.as_ref().is_some_and(|route| route
        .route_reason
        .contains("direct_answer_gate_background_only_ignored")));
}

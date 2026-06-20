#[test]
fn direct_answer_gate_workspace_summary_after_sanitized_freeform_rewrite_stays_direct() {
    let mut route = chat_route_for_gate();
    route.route_reason =
        "untrusted_normalizer_freeform_rewrite_removed_from_execution_context".to_string();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let gate = gate_out(
        "planner_execute",
        gate_contract(true, "current_workspace", "workspace_project_summary"),
    );

    let preflight = apply_direct_answer_gate_outcome(
        &crate::AppState::test_default_with_fixture_provider(),
        &mut ctx,
        "Draft a standalone user-facing text artifact.",
        gate,
    );

    assert!(matches!(preflight, DirectAnswerPreflight::DirectAnswer));
    assert!(ctx.route_result.as_ref().is_some_and(|route| route
        .route_reason
        .contains("direct_answer_gate_sanitized_freeform_promotion_ignored")));
}

#[test]
fn direct_answer_gate_path_contract_after_sanitized_freeform_rewrite_stays_direct() {
    let mut route = chat_route_for_gate();
    route.route_reason =
        "untrusted_normalizer_freeform_rewrite_removed_from_execution_context".to_string();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut gate = gate_out("planner_execute", gate_contract(true, "path", "none"));
    gate.reference_resolution.target = "missing_locator".to_string();

    let preflight = apply_direct_answer_gate_outcome(
        &crate::AppState::test_default_with_fixture_provider(),
        &mut ctx,
        "Write a standalone test plan.",
        gate,
    );

    assert!(matches!(preflight, DirectAnswerPreflight::DirectAnswer));
    assert!(ctx.route_result.as_ref().is_some_and(|route| route
        .route_reason
        .contains("direct_answer_gate_sanitized_freeform_promotion_ignored")));
}

#[test]
fn direct_answer_gate_keeps_deictic_file_followup_promotable() {
    let mut route = chat_route_for_gate();
    route.resolved_intent =
        "User wants the selected file.\nanswer_candidate: README.md".to_string();
    let promoted_contract = crate::IntentOutputContract {
        requires_content_evidence: true,
        delivery_required: true,
        locator_kind: crate::OutputLocatorKind::Path,
        delivery_intent: crate::OutputDeliveryIntent::FileSingle,
        locator_hint: "README.md".to_string(),
        ..crate::IntentOutputContract::default()
    };

    assert!(
        !direct_answer_gate_promotion_depends_only_on_background_context(
            &crate::AppState::test_default_with_fixture_provider(),
            "Send that file.",
            &route,
            &promoted_contract,
            &DirectAnswerGateReferenceResolutionOut {
                target: "current_action_result".to_string(),
            },
            false,
        )
    );
}

#[test]
fn recent_file_context_promotion_ignores_sentence_punctuation() {
    let mut route = chat_route_for_gate();
    route.resolved_intent = concat!(
            "Acknowledge that no concrete target is bound.\n",
            "answer_candidate: Understood. No file read triggered. If you need a specific path, name it."
        )
        .to_string();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        cross_turn_recent_execution_context: Some(
            "### RECENT_EXECUTION_EVENTS\n\
                 - ts=2 kind=ask request=read configs/config.toml result=ok\n\
                 - ts=1 kind=ask request=read README.md result=ok"
                .to_string(),
        ),
        ..Default::default()
    };
    let gate = gate_out("direct_answer", gate_contract(false, "none", "none"));

    let outcome = apply_direct_answer_gate_outcome(
        &crate::AppState::test_default_with_fixture_provider(),
        &mut ctx,
        "Acknowledge only; no current target is bound.",
        gate,
    );

    assert!(matches!(outcome, DirectAnswerPreflight::DirectAnswer));
    let route = ctx.route_result.expect("route");
    assert!(!route.is_execute_gate());
    assert!(!route
        .route_reason
        .contains("direct_answer_gate_recent_file_context_execute"));
}

#[test]
fn direct_answer_gate_promotes_artifact_candidate_with_recent_file_targets_to_planner() {
    let root = TempDirGuard::new("recent_file_targets");
    let readme = root.path.join("README.md");
    let notes = root.path.join("service_notes.md");
    std::fs::write(&readme, "# Demo\nmentions app_config.toml\n").expect("write readme");
    std::fs::write(&notes, "# Service\nrestart notes\n").expect("write notes");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.path.clone();

    let mut route = chat_route_for_gate();
    route.resolved_intent = concat!(
        "Compare the previous file targets in one sentence\n",
        "answer_candidate: app_config.toml is config; service_notes.md is service notes"
    )
    .to_string();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        cross_turn_recent_execution_context: Some(format!(
            "### RECENT_EXECUTION_EVENTS\n\
                 - ts=2 kind=ask request=read {} result=- `app_config.toml`: sample config\n\
                 - ts=1 kind=ask request=read {} result=service restart notes",
            readme.display(),
            notes.display()
        )),
        ..Default::default()
    };
    let gate = gate_out("direct_answer", gate_contract(false, "none", "none"));

    let outcome =
        apply_direct_answer_gate_outcome(&state, &mut ctx, "compare the recent files", gate);

    assert!(matches!(outcome, DirectAnswerPreflight::PlannerExecute(_)));
    let route = ctx.route_result.expect("route");
    assert!(route.is_execute_gate());
    assert!(route.output_contract.requires_content_evidence);
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::CurrentWorkspace
    );
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::ContentExcerptSummary
    );
    assert!(route
        .route_reason
        .contains("direct_answer_gate_recent_file_context_execute"));
}

#[test]
fn direct_answer_gate_keeps_recent_file_basename_candidate_direct() {
    let root = TempDirGuard::new("recent_file_basename_candidate");
    let readme = root.path.join("README.md");
    let notes = root.path.join("service_notes.md");
    std::fs::write(&readme, "# RustClaw\n\nProject description\n").expect("write readme");
    std::fs::write(&notes, "# Service Notes\n\nFixture notes\n").expect("write notes");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.path.clone();

    let mut route = chat_route_for_gate();
    route.resolved_intent = concat!(
        "Identify which recent file target is more like a project description.\n",
        "answer_candidate: README.md"
    )
    .to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        cross_turn_recent_execution_context: Some(format!(
            "### RECENT_EXECUTION_EVENTS\n\
                 - ts=2 kind=ask request=read {} result=# Service Notes\n\
                 - ts=1 kind=ask request=read {} result=# RustClaw",
            notes.display(),
            readme.display()
        )),
        ..Default::default()
    };
    let mut gate = gate_out("direct_answer", gate_contract(true, "none", "none"));
    gate.output_contract.response_shape = "scalar".to_string();

    let outcome = apply_direct_answer_gate_outcome(
        &state,
        &mut ctx,
        "which recent file is more like a project description; answer only the file name",
        gate,
    );

    assert!(matches!(outcome, DirectAnswerPreflight::DirectAnswer));
    let route = ctx.route_result.expect("route");
    assert!(route.is_chat_gate());
    assert!(route
        .route_reason
        .contains("direct_answer_gate_bound_candidate_evidence"));
}

#[test]
fn clarify_recent_execution_judgment_context_promotes_to_chat() {
    let root = TempDirGuard::new("clarify_recent_execution_judgment");
    let readme = root.path.join("README.md");
    let notes = root.path.join("service_notes.md");
    std::fs::write(&readme, "# RustClaw\n\nProject description\n").expect("write readme");
    std::fs::write(&notes, "# Service Notes\n\nFixture notes\n").expect("write notes");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.path.clone();

    let mut route = chat_route_for_gate();
    route.set_clarify_gate();
    route.needs_clarify = true;
    route.route_reason =
        "semantic_contract_requires_evidence; clarify_reason_code:missing_read_target".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = root.path.display().to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExcerptKindJudgment;
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        cross_turn_recent_execution_context: Some(format!(
            "### RECENT_EXECUTION_EVENTS\n\
                 - ts=2 kind=ask request=read {} result=# Service Notes\n\
                 - ts=1 kind=ask request=read {} result=# RustClaw",
            notes.display(),
            readme.display()
        )),
        ..Default::default()
    };

    assert!(promote_clarify_recent_execution_judgment_context_to_chat(
        &state,
        Some(&mut ctx),
    ));
    let route = ctx.route_result.as_ref().expect("route");
    assert!(!route.needs_clarify);
    assert!(route.is_chat_gate());
    assert!(route
        .route_reason
        .contains("clarify_recent_execution_judgment_to_chat"));
    assert!(direct_answer_gate_can_skip_for_recent_execution_judgment_context(Some(&ctx)));
}

#[test]
fn config_risk_assessment_default_main_config_promotes_clarify_to_planner() {
    let root = TempDirGuard::new("config_risk_default_main_config");
    std::fs::create_dir_all(root.path.join("configs")).expect("create configs dir");
    std::fs::write(
        root.path.join("configs/config.toml"),
        "selected_vendor = \"minimax\"\n",
    )
    .expect("write config");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.path.clone();

    let mut route = chat_route_for_gate();
    route.set_clarify_gate();
    route.needs_clarify = true;
    route.clarify_question = "missing locator".to_string();
    route.route_reason =
        "semantic_contract_requires_evidence; clarify_reason_code:missing_read_target".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = root.path.join("rustclaw").display().to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ConfigRiskAssessment;
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert!(
        promote_clarify_config_risk_assessment_default_config_to_planner(
            &state,
            "audit this product configuration without exposing secret values",
            Some(&mut ctx),
        )
    );
    let route = ctx.route_result.as_ref().expect("route");
    assert!(!route.needs_clarify);
    assert!(route.is_execute_gate());
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Path
    );
    assert_eq!(route.output_contract.locator_hint, "configs/config.toml");
    assert_eq!(
        route.output_contract.delivery_intent,
        crate::OutputDeliveryIntent::None
    );
    assert!(route
        .route_reason
        .contains("config_risk_default_main_config_to_planner"));
}

#[test]
fn config_risk_assessment_default_main_config_keeps_explicit_locator_clarify() {
    let root = TempDirGuard::new("config_risk_explicit_locator");
    std::fs::create_dir_all(root.path.join("configs")).expect("create configs dir");
    std::fs::write(root.path.join("configs/config.toml"), "enabled = true\n")
        .expect("write config");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.path.clone();

    let mut route = chat_route_for_gate();
    route.set_clarify_gate();
    route.needs_clarify = true;
    route.route_reason = "clarify_reason_code:missing_read_target".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ConfigRiskAssessment;
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert!(
        !promote_clarify_config_risk_assessment_default_config_to_planner(
            &state,
            "audit configs/other.toml",
            Some(&mut ctx),
        )
    );
    let route = ctx.route_result.as_ref().expect("route");
    assert!(route.needs_clarify);
    assert!(route.is_clarify_gate());
}

#[test]
fn direct_answer_gate_context_marks_answer_candidate_as_unobserved() {
    let mut route = chat_route_for_gate();
    route.resolved_intent = "get current runtime scalar\nanswer_candidate: stale_value".to_string();
    route.route_reason = "prior normalizer said direct answer".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let context = direct_answer_gate_route_context(Some(&ctx));

    assert!(context.contains("### PRIOR_ROUTE_CONTEXT"));
    assert!(context.contains("resolved_user_intent: get current runtime scalar"));
    assert!(context.contains("normalizer_answer_candidate_present: true"));
    assert!(context.contains("not runtime evidence"));
    assert!(context.contains("prior_route_reason: prior normalizer said direct answer"));
    assert!(!context.contains("stale_value"));
    assert!(!context.contains("answer_candidate: stale_value"));
}

#[test]
fn direct_answer_gate_recent_execution_context_exposes_targets_not_excerpt_paths() {
    let ctx = crate::agent_engine::AgentRunContext {
            cross_turn_recent_execution_context: Some(
                "### RECENT_EXECUTION_EVENTS\n- request=read /tmp/README.md result=- `/tmp/config.toml`: sample config"
                    .to_string(),
            ),
            ..Default::default()
        };

    let context = direct_answer_gate_recent_execution_context(Some(&ctx));

    assert!(context.contains("### RECENT_EXECUTION_CONTEXT"));
    assert!(context.contains("Previous executed targets are authoritative"));
    assert!(context.contains("Paths mentioned inside a prior file excerpt are content"));
    assert!(context.contains("/tmp/README.md"));
    assert!(context.contains("/tmp/config.toml"));
}

#[test]
fn direct_answer_gate_promotes_contract_evidence_even_when_decision_is_direct() {
    let route = chat_route_for_gate();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut contract = gate_contract(true, "path", "content_excerpt_summary");
    contract.locator_hint = "/tmp/clawd.log".to_string();
    let gate = gate_out("direct_answer", contract);
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome =
        apply_direct_answer_gate_outcome(&state, &mut ctx, "summarize /tmp/clawd.log", gate);

    assert!(matches!(outcome, DirectAnswerPreflight::PlannerExecute(_)));
    let route = ctx.route_result.expect("route");
    assert_eq!(
        route.ask_mode,
        crate::AskMode::planner_execute_chat_wrapped()
    );
    assert!(route.is_execute_gate());
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Path
    );
    assert_eq!(route.output_contract.locator_hint, "/tmp/clawd.log");
    assert!(route
        .route_reason
        .contains("direct_answer_gate_contract_execute"));
}

#[test]
fn direct_answer_gate_binds_resolvable_workspace_child_locator() {
    let root = TempDirGuard::new("gate_workspace_child");
    std::fs::create_dir_all(root.path.join("docs")).expect("create docs");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.path.clone();
    state.skill_rt.default_locator_search_dir = root.path.clone();
    let route = chat_route_for_gate();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut contract = gate_contract(true, "path", "content_excerpt_summary");
    contract.locator_hint = "docs".to_string();
    let gate = gate_out("planner_execute", contract);

    let outcome =
        apply_direct_answer_gate_outcome(&state, &mut ctx, "look at the docs folder", gate);

    assert!(matches!(outcome, DirectAnswerPreflight::PlannerExecute(_)));
    let route = ctx.route_result.expect("route");
    assert!(route.is_execute_gate());
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Path
    );
    assert_eq!(
        route.output_contract.locator_hint,
        root.path.join("docs").display().to_string()
    );
}

#[test]
fn direct_answer_gate_binds_deictic_request_when_request_itself_resolves_target() {
    let root = TempDirGuard::new("gate_deictic_workspace_child");
    std::fs::create_dir_all(root.path.join("docs")).expect("create docs");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.path.clone();
    state.skill_rt.default_locator_search_dir = root.path.clone();
    let route = chat_route_for_gate();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut contract = gate_contract(true, "path", "content_excerpt_summary");
    contract.locator_hint = "docs".to_string();
    let gate = gate_out("planner_execute", contract);

    let outcome = apply_direct_answer_gate_outcome(
        &state,
        &mut ctx,
        "look at the docs folder and summarize it",
        gate,
    );

    assert!(matches!(outcome, DirectAnswerPreflight::PlannerExecute(_)));
    let route = ctx.route_result.expect("route");
    assert_eq!(
        route.output_contract.locator_hint,
        root.path.join("docs").display().to_string()
    );
}

#[test]
fn direct_answer_gate_clarifies_unbound_deictic_observation_instead_of_guessing_locator() {
    let mut route = chat_route_for_gate();
    route.resolved_intent = "获取指定文件中 name 字段的值\nanswer_candidate: rustclaw".to_string();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut contract = gate_contract(true, "path", "structured_keys");
    contract.locator_hint = "Cargo.toml".to_string();
    let mut gate = gate_out("planner_execute", contract);
    gate.reference_resolution.target = "unresolved_prior_object".to_string();
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome = apply_direct_answer_gate_outcome(
        &state,
        &mut ctx,
        "读一下那个文件里的名字字段，只输出值",
        gate,
    );

    assert!(matches!(outcome, DirectAnswerPreflight::Clarify(_)));
    let route = ctx.route_result.expect("route");
    assert_eq!(route.ask_mode, crate::AskMode::clarify());
    assert!(route.needs_clarify);
    assert!(route
        .route_reason
        .contains("direct_answer_gate_unbound_deictic_clarify"));
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(route.output_contract.locator_hint.is_empty());
}

#[test]
fn direct_answer_gate_allows_deictic_observation_with_structured_auto_locator() {
    let route = chat_route_for_gate();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        auto_locator_path: Some("/tmp/bound/package.json".to_string()),
        ..Default::default()
    };
    let mut contract = gate_contract(true, "path", "structured_keys");
    contract.locator_hint = "/tmp/bound/package.json".to_string();
    let mut gate = gate_out("planner_execute", contract);
    gate.reference_resolution.target = "unresolved_prior_object".to_string();
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome = apply_direct_answer_gate_outcome(
        &state,
        &mut ctx,
        "读一下那个文件里的名字字段，只输出值",
        gate,
    );

    assert!(matches!(outcome, DirectAnswerPreflight::PlannerExecute(_)));
    let route = ctx.route_result.expect("route");
    assert!(route.is_execute_gate());
    assert_eq!(
        route.output_contract.locator_hint,
        "/tmp/bound/package.json"
    );
}

#[test]
fn direct_answer_gate_clarifies_deictic_observation_with_gate_locator_hint_only() {
    let route = chat_route_for_gate();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut contract = gate_contract(true, "path", "none");
    contract.locator_hint = "/tmp/bound/README.md".to_string();
    let mut gate = gate_out("planner_execute", contract);
    gate.reference_resolution.target = "unresolved_prior_object".to_string();
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome =
        apply_direct_answer_gate_outcome(&state, &mut ctx, "把那个文件开头读 10 行", gate);

    assert!(matches!(outcome, DirectAnswerPreflight::Clarify(_)));
    let route = ctx.route_result.expect("route");
    assert!(route.needs_clarify);
    assert!(route
        .route_reason
        .contains("direct_answer_gate_unbound_deictic_clarify"));
    assert!(route.output_contract.locator_hint.is_empty());
}

#[test]
fn direct_answer_gate_clarifies_claimed_current_locator_without_current_surface() {
    let route = chat_route_for_gate();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut contract = gate_contract(true, "path", "none");
    contract.locator_hint =
        "/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/README.md".to_string();
    let mut gate = gate_out("planner_execute", contract);
    gate.reference_resolution.target = "current_turn_locator".to_string();
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome = apply_direct_answer_gate_outcome(&state, &mut ctx, "读一下那个文件前 3 行", gate);

    assert!(matches!(outcome, DirectAnswerPreflight::Clarify(_)));
    let route = ctx.route_result.expect("route");
    assert!(route.needs_clarify);
    assert!(route
        .route_reason
        .contains("direct_answer_gate_unbound_deictic_clarify"));
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(route.output_contract.locator_hint.is_empty());
}

#[test]
fn direct_answer_gate_clarifies_locator_hint_without_current_surface_or_reference_report() {
    let route = chat_route_for_gate();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut contract = gate_contract(true, "path", "none");
    contract.locator_hint =
        "/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/README.md".to_string();
    let gate = gate_out("planner_execute", contract);
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome = apply_direct_answer_gate_outcome(&state, &mut ctx, "读一下那个文件前 3 行", gate);

    assert!(matches!(outcome, DirectAnswerPreflight::Clarify(_)));
    let route = ctx.route_result.expect("route");
    assert!(route.needs_clarify);
    assert!(route
        .route_reason
        .contains("direct_answer_gate_unbound_deictic_clarify"));
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(route.output_contract.locator_hint.is_empty());
}

#[test]
fn direct_answer_gate_allows_deictic_observation_with_authoritative_anchor() {
    let route = chat_route_for_gate();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        has_authoritative_deictic_anchor: true,
        ..Default::default()
    };
    let mut contract = gate_contract(true, "path", "none");
    contract.locator_hint = "/tmp/bound/README.md".to_string();
    let mut gate = gate_out("planner_execute", contract);
    gate.reference_resolution.target = "unresolved_prior_object".to_string();
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome =
        apply_direct_answer_gate_outcome(&state, &mut ctx, "把那个文件开头读 10 行", gate);

    assert!(matches!(outcome, DirectAnswerPreflight::PlannerExecute(_)));
    let route = ctx.route_result.expect("route");
    assert!(route.is_execute_gate());
    assert_eq!(route.output_contract.locator_hint, "/tmp/bound/README.md");
    assert!(route.output_contract.requires_content_evidence);
}

#[test]
fn direct_answer_gate_allows_current_workspace_summary_with_deictic_surface() {
    let route = chat_route_for_gate();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let gate = gate_out(
        "planner_execute",
        gate_contract(true, "current_workspace", "workspace_project_summary"),
    );
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome = apply_direct_answer_gate_outcome(
        &state,
        &mut ctx,
        "先看当前目录顶层主要文件夹，再用一句话解释这个仓库怎么分区",
        gate,
    );

    assert!(matches!(outcome, DirectAnswerPreflight::PlannerExecute(_)));
    let route = ctx.route_result.expect("route");
    assert!(route.is_execute_gate());
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::CurrentWorkspace
    );
    assert!(!direct_answer_gate_promotion_needs_unbound_deictic_clarify(
        &state,
        "先看当前目录顶层主要文件夹，再用一句话解释这个仓库怎么分区",
        None,
        false,
        false,
        &crate::IntentOutputContract {
            requires_content_evidence: true,
            locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
            semantic_kind: crate::OutputSemanticKind::None,
            ..Default::default()
        },
        &DirectAnswerGateReferenceResolutionOut {
            target: "current_action_result".to_string(),
        },
    ));
}

#[test]
fn direct_answer_gate_keeps_active_file_basename_answer_candidate_direct() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let mut route = chat_route_for_gate();
    route.resolved_intent =
        "用户请求仅返回当前文件的文件名（basename）\nanswer_candidate: README.md".to_string();
    route.route_reason = "active_file_basename_answer_candidate_direct".to_string();
    route.output_contract = crate::IntentOutputContract {
        response_shape: crate::OutputResponseShape::Scalar,
        requires_content_evidence: false,
        delivery_required: false,
        locator_kind: crate::OutputLocatorKind::None,
        delivery_intent: crate::OutputDeliveryIntent::None,
        semantic_kind: crate::OutputSemanticKind::FileBasename,
        ..Default::default()
    };
    let gate = gate_out(
        "direct_answer",
        gate_contract(false, "none", "file_basename"),
    );

    assert!(!direct_answer_gate_candidate_needs_unbound_context_clarify(
        &state,
        "只说这个文件名",
        &route,
        &gate,
        None,
        false,
        false,
        false,
    ));
}

#[test]
fn direct_answer_gate_clarifies_current_workspace_when_reference_is_unbound() {
    let route = chat_route_for_gate();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut gate = gate_out(
        "planner_execute",
        gate_contract(true, "current_workspace", "content_presence_check"),
    );
    gate.reference_resolution.target = "missing_locator".to_string();
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome =
        apply_direct_answer_gate_outcome(&state, &mut ctx, "查看指定 schema 的 enum", gate);

    assert!(matches!(outcome, DirectAnswerPreflight::Clarify(_)));
    let route = ctx.route_result.expect("route");
    assert!(route.needs_clarify);
    assert!(route
        .route_reason
        .contains("direct_answer_gate_unbound_deictic_clarify"));
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(route.output_contract.locator_hint.is_empty());
}

#[test]
fn recent_count_comparison_uses_completed_count_inventory_tasks() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let user_id = 7;
    let chat_id = 9;
    let user_key = "user-key";
    insert_count_inventory_task(
        &state,
        "count-scripts",
        user_id,
        chat_id,
        user_key,
        "scripts",
        64,
        "2026-05-18T08:00:00Z",
    );
    insert_count_inventory_task(
        &state,
        "count-document",
        user_id,
        chat_id,
        user_key,
        "/tmp/repo/document",
        34,
        "2026-05-18T08:01:00Z",
    );
    let task = crate::ClaimedTask {
        task_id: "compare-current".to_string(),
        user_id,
        chat_id,
        user_key: Some(user_key.to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: serde_json::json!({"text":"上一个和上上个哪个更多，只回答目录名"})
            .to_string(),
    };
    let mut route = chat_route_for_gate();
    route.ask_mode = crate::AskMode::planner_execute_plain();
    route.resolved_intent =
            "Compare the two most recent count_inventory observations and report the selected target label."
                .to_string();
    route.route_reason = "structured_quantity_comparison".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.output_contract.requires_content_evidence = true;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskRequest),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
            should_interrupt_active_run: false,
            state_patch: Some(serde_json::json!({
                "quantity_comparison": {
                    "selection": "max",
                    "source": "recent_count_inventory"
                }
            })),
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    assert_eq!(
        recent_count_comparison_direct_answer(
            &state,
            &task,
            "上一个和上上个哪个更多，只回答目录名",
            Some(&ctx),
        )
        .as_deref(),
        Some("scripts")
    );
}

#[test]
fn recent_count_comparison_uses_wrapped_count_inventory_tasks() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let user_id = 67;
    let chat_id = 69;
    let user_key = "user-key";
    insert_wrapped_count_inventory_task(
        &state,
        "wrapped-count-scripts",
        user_id,
        chat_id,
        user_key,
        "scripts",
        65,
        "2026-05-18T08:00:00Z",
    );
    insert_wrapped_count_inventory_task(
        &state,
        "wrapped-count-document",
        user_id,
        chat_id,
        user_key,
        "/tmp/repo/document",
        37,
        "2026-05-18T08:01:00Z",
    );
    let task = crate::ClaimedTask {
        task_id: "compare-wrapped-current".to_string(),
        user_id,
        chat_id,
        user_key: Some(user_key.to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: serde_json::json!({"text":"compare the last two count observations"})
            .to_string(),
    };
    let mut route = chat_route_for_gate();
    route.ask_mode = crate::AskMode::direct_answer();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskRequest),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
            should_interrupt_active_run: false,
            state_patch: Some(serde_json::json!({
                "quantity_comparison": {
                    "selection": "max",
                    "source": "recent_count_inventory"
                }
            })),
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    assert_eq!(
        recent_count_comparison_direct_answer(
            &state,
            &task,
            "compare the last two count observations",
            Some(&ctx),
        )
        .as_deref(),
        Some("scripts")
    );
}

#[test]
fn recent_count_comparison_uses_structured_selection_when_route_semantic_is_none() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let user_id = 97;
    let chat_id = 99;
    let user_key = "user-key";
    insert_wrapped_count_inventory_task(
        &state,
        "semantic-none-count-scripts",
        user_id,
        chat_id,
        user_key,
        "scripts",
        65,
        "2026-05-18T08:00:00Z",
    );
    insert_wrapped_count_inventory_task(
        &state,
        "semantic-none-count-document",
        user_id,
        chat_id,
        user_key,
        "document",
        37,
        "2026-05-18T08:01:00Z",
    );
    let task = crate::ClaimedTask {
        task_id: "compare-semantic-none-current".to_string(),
        user_id,
        chat_id,
        user_key: Some(user_key.to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: serde_json::json!({"text":"compare the last two count observations"})
            .to_string(),
    };
    let mut route = chat_route_for_gate();
    route.ask_mode = crate::AskMode::direct_answer();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskCorrect),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
            should_interrupt_active_run: false,
            state_patch: Some(serde_json::json!({
                "quantity_comparison": {
                    "selection": "max",
                    "source": "recent_count_inventory"
                }
            })),
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    assert_eq!(
        recent_count_comparison_direct_answer(
            &state,
            &task,
            "compare the last two count observations",
            Some(&ctx),
        )
        .as_deref(),
        Some("scripts")
    );
}

#[test]
fn direct_answer_gate_quantity_state_patch_stays_direct_and_feeds_count_comparison() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let user_id = 77;
    let chat_id = 79;
    let user_key = "user-key";
    insert_wrapped_count_inventory_task(
        &state,
        "gate-count-scripts",
        user_id,
        chat_id,
        user_key,
        "scripts",
        65,
        "2026-05-18T08:00:00Z",
    );
    insert_wrapped_count_inventory_task(
        &state,
        "gate-count-document",
        user_id,
        chat_id,
        user_key,
        "document",
        37,
        "2026-05-18T08:01:00Z",
    );
    let task = crate::ClaimedTask {
        task_id: "compare-gate-state-patch".to_string(),
        user_id,
        chat_id,
        user_key: Some(user_key.to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: serde_json::json!({"text":"compare the last two count observations"})
            .to_string(),
    };
    let route = chat_route_for_gate();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let gate = DirectAnswerGateOut {
        decision: "direct_answer".to_string(),
        reason: "observed_recent_count_comparison".to_string(),
        confidence: 0.95,
        clarify_question: String::new(),
        resolved_user_intent:
            "Compare two recent count_inventory observations and return only the selected label."
                .to_string(),
        reference_resolution: DirectAnswerGateReferenceResolutionOut {
            target: "comparison_result".to_string(),
        },
        output_contract: DirectAnswerGateContractOut {
            response_shape: "scalar".to_string(),
            exact_sentence_count: None,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: "none".to_string(),
            delivery_intent: "none".to_string(),
            semantic_kind: "quantity_comparison".to_string(),
            locator_hint: String::new(),
            self_extension: DirectAnswerGateSelfExtensionOut::default(),
        },
        state_patch: Some(serde_json::json!({
            "quantity_comparison": {
                "selection": "max",
                "source": "recent_count_inventory",
                "candidates": [
                    {"label": "scripts", "count": 65},
                    {"label": "document", "count": 37}
                ],
                "winner": "scripts"
            }
        })),
    };

    let outcome = apply_direct_answer_gate_outcome(
        &state,
        &mut ctx,
        "compare the last two count observations",
        gate,
    );

    assert!(matches!(outcome, DirectAnswerPreflight::DirectAnswer));
    assert_eq!(
        ctx.route_result
            .as_ref()
            .map(|route| route.output_contract.semantic_kind),
        Some(crate::OutputSemanticKind::QuantityComparison)
    );
    assert_eq!(
        ctx.turn_analysis
            .as_ref()
            .and_then(|analysis| analysis.state_patch.as_ref())
            .and_then(|patch| patch.pointer("/quantity_comparison/selection"))
            .and_then(serde_json::Value::as_str),
        Some("max")
    );
    assert_eq!(
        recent_count_comparison_direct_answer(
            &state,
            &task,
            "compare the last two count observations",
            Some(&ctx),
        )
        .as_deref(),
        Some("scripts")
    );
}

#[test]
fn recent_count_comparison_overrides_bad_direct_answer_candidate() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let user_id = 17;
    let chat_id = 19;
    let user_key = "user-key";
    insert_count_inventory_task(
        &state,
        "count-scripts",
        user_id,
        chat_id,
        user_key,
        "scripts",
        64,
        "2026-05-18T08:00:00Z",
    );
    insert_count_inventory_task(
        &state,
        "count-document",
        user_id,
        chat_id,
        user_key,
        "/tmp/repo/document",
        34,
        "2026-05-18T08:01:00Z",
    );
    let task = crate::ClaimedTask {
        task_id: "compare-direct".to_string(),
        user_id,
        chat_id,
        user_key: Some(user_key.to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: serde_json::json!({"text":"上一个和上上个哪个更多，只回答目录名"})
            .to_string(),
    };
    let mut route = chat_route_for_gate();
    route.ask_mode = crate::AskMode::direct_answer();
    route.resolved_intent =
            "Compare the two observed count_inventory totals and return only the selected target label.\nanswer_candidate: 当前范围"
                .to_string();
    route.route_reason = "structured_quantity_comparison".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskRequest),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
            should_interrupt_active_run: false,
            state_patch: Some(serde_json::json!({
                "quantity_comparison": {
                    "selection": "max",
                    "source": "recent_count_inventory"
                }
            })),
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    assert_eq!(
        recent_count_comparison_direct_answer(
            &state,
            &task,
            "上一个和上上个哪个更多，只回答目录名",
            Some(&ctx),
        )
        .as_deref(),
        Some("scripts")
    );
}

#[test]
fn recent_count_comparison_uses_min_selection_from_state_patch() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let user_id = 27;
    let chat_id = 29;
    let user_key = "user-key";
    insert_count_inventory_task(
        &state,
        "count-scripts",
        user_id,
        chat_id,
        user_key,
        "scripts",
        64,
        "2026-05-18T08:00:00Z",
    );
    insert_count_inventory_task(
        &state,
        "count-document",
        user_id,
        chat_id,
        user_key,
        "/tmp/repo/document",
        34,
        "2026-05-18T08:01:00Z",
    );
    let task = crate::ClaimedTask {
        task_id: "compare-direct-min".to_string(),
        user_id,
        chat_id,
        user_key: Some(user_key.to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: serde_json::json!({"text":"上一个和上上个哪个更多，只回答目录名"})
            .to_string(),
    };
    let mut route = chat_route_for_gate();
    route.ask_mode = crate::AskMode::direct_answer();
    route.resolved_intent =
            "Compare the two observed count_inventory totals and return only the selected target label."
                .to_string();
    route.route_reason = "structured_quantity_comparison".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskRequest),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
            should_interrupt_active_run: false,
            state_patch: Some(serde_json::json!({
                "quantity_comparison": {
                    "selection": "min",
                    "source": "recent_count_inventory"
                }
            })),
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    assert_eq!(
        recent_count_comparison_direct_answer(
            &state,
            &task,
            "上一个和上上个哪个更多，只回答目录名",
            Some(&ctx),
        )
        .as_deref(),
        Some("document")
    );
}

#[test]
fn recent_count_comparison_ignores_missing_structured_selection() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let user_id = 37;
    let chat_id = 39;
    let user_key = "user-key";
    insert_count_inventory_task(
        &state,
        "count-scripts",
        user_id,
        chat_id,
        user_key,
        "scripts",
        64,
        "2026-05-18T08:00:00Z",
    );
    insert_count_inventory_task(
        &state,
        "count-document",
        user_id,
        chat_id,
        user_key,
        "/tmp/repo/document",
        34,
        "2026-05-18T08:01:00Z",
    );
    let task = crate::ClaimedTask {
        task_id: "compare-missing-selection".to_string(),
        user_id,
        chat_id,
        user_key: Some(user_key.to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: serde_json::json!({"text":"上一个和上上个哪个更多，只回答目录名"})
            .to_string(),
    };
    let mut route = chat_route_for_gate();
    route.ask_mode = crate::AskMode::direct_answer();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert!(recent_count_comparison_direct_answer(
        &state,
        &task,
        "上一个和上上个哪个更多，只回答目录名",
        Some(&ctx),
    )
    .is_none());
}

#[test]
fn direct_answer_gate_does_not_skip_recent_count_scalar_context_without_structured_selection() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let user_id = 47;
    let chat_id = 49;
    let user_key = "user-key";
    insert_count_inventory_task(
        &state,
        "count-scripts",
        user_id,
        chat_id,
        user_key,
        "scripts",
        64,
        "2026-05-18T08:00:00Z",
    );
    insert_count_inventory_task(
        &state,
        "count-document",
        user_id,
        chat_id,
        user_key,
        "/tmp/repo/document",
        34,
        "2026-05-18T08:01:00Z",
    );
    let task = crate::ClaimedTask {
        task_id: "compare-skip-gate".to_string(),
        user_id,
        chat_id,
        user_key: Some(user_key.to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: serde_json::json!({"text":"compare the two recent counts, scalar only"})
            .to_string(),
    };
    let mut route = chat_route_for_gate();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert!(!direct_answer_gate_can_skip_for_recent_count_context(
        &state,
        &task,
        Some(&ctx),
    ));
}

#[test]
fn direct_answer_gate_does_not_skip_recent_count_context_without_structured_selection_even_when_shape_is_free(
) {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let user_id = 57;
    let chat_id = 59;
    let user_key = "user-key";
    insert_count_inventory_task(
        &state,
        "count-scripts",
        user_id,
        chat_id,
        user_key,
        "scripts",
        65,
        "2026-05-18T08:00:00Z",
    );
    insert_count_inventory_task(
        &state,
        "count-document",
        user_id,
        chat_id,
        user_key,
        "document",
        36,
        "2026-05-18T08:01:00Z",
    );
    let task = crate::ClaimedTask {
        task_id: "compare-skip-gate-free".to_string(),
        user_id,
        chat_id,
        user_key: Some(user_key.to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: serde_json::json!({"text":"compare the two recent counts"}).to_string(),
    };
    let mut route = chat_route_for_gate();
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert!(!direct_answer_gate_can_skip_for_recent_count_context(
        &state,
        &task,
        Some(&ctx),
    ));
}

#[test]
fn direct_answer_gate_skips_recent_count_context_when_deterministic_answer_is_ready() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let user_id = 87;
    let chat_id = 89;
    let user_key = "user-key";
    insert_wrapped_count_inventory_task(
        &state,
        "count-scripts-selected",
        user_id,
        chat_id,
        user_key,
        "scripts",
        65,
        "2026-05-18T08:00:00Z",
    );
    insert_wrapped_count_inventory_task(
        &state,
        "count-document-selected",
        user_id,
        chat_id,
        user_key,
        "document",
        36,
        "2026-05-18T08:01:00Z",
    );
    let task = crate::ClaimedTask {
        task_id: "compare-skip-gate-selected".to_string(),
        user_id,
        chat_id,
        user_key: Some(user_key.to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: serde_json::json!({"text":"compare the two recent counts"}).to_string(),
    };
    let mut route = chat_route_for_gate();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskRequest),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
            should_interrupt_active_run: false,
            state_patch: Some(serde_json::json!({
                "quantity_comparison": {
                    "selection": "max",
                    "source": "recent_count_inventory"
                }
            })),
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    assert!(direct_answer_gate_can_skip_for_recent_count_context(
        &state,
        &task,
        Some(&ctx),
    ));
}

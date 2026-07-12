use super::{
    agent_loop_boundary_observations_block, agent_loop_default_context,
    apply_post_route_refinements, build_agent_run_context_from_prepared_flow, PreparedAskFlow,
};

fn base_route() -> crate::RouteResult {
    crate::RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "inspect workspace".to_string(),
        needs_clarify: false,
        route_reason: "test_route".to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: "RustClaw".to_string(),
        output_contract: crate::IntentOutputContract::default(),
    }
}

fn prepared_flow_with_context() -> PreparedAskFlow {
    PreparedAskFlow {
        context_bundle_summary: "context-summary".to_string(),
        memory_trace: None,
        route_result: base_route(),
        execution_recipe_hint: None,
        execution_recipe_plan_hint: None,
        turn_analysis: None,
        boundary_envelope: Some(crate::intent_router::BoundaryEnvelope {
            raw_chars: "raw user request".chars().count(),
            explicit_locators: vec!["README.md".to_string()],
            ..Default::default()
        }),
        clarify_fallback_source: None,
        auto_locator_path: Some("/tmp/workspace/README.md".to_string()),
        resolved_prompt_for_execution: "resolved execution prompt".to_string(),
        prompt_with_memory_for_execution: "memory + resolved execution prompt".to_string(),
        recent_execution_context: "recent execution facts".to_string(),
        session_alias_bindings: Vec::new(),
        ask_mode: crate::AskMode::act_plain(),
        fuzzy_locator_suggestions: vec!["README.md".to_string()],
        should_route_schedule_direct: false,
    }
}

#[test]
fn prepared_ask_flow_builds_agent_run_context_for_replay_boundary() {
    let prepared_flow = prepared_flow_with_context();
    let ctx = build_agent_run_context_from_prepared_flow("raw user request", &prepared_flow);

    assert_eq!(
        ctx.original_user_request.as_deref(),
        Some("raw user request")
    );
    assert_eq!(
        ctx.user_request.as_deref(),
        Some("resolved execution prompt")
    );
    assert_eq!(
        ctx.context_bundle_summary.as_deref(),
        Some("context-summary")
    );
    assert_eq!(
        ctx.auto_locator_path.as_deref(),
        Some("/tmp/workspace/README.md")
    );
    assert_eq!(
        ctx.cross_turn_recent_execution_context.as_deref(),
        Some("recent execution facts")
    );
    assert_eq!(
        ctx.boundary_envelope
            .as_ref()
            .map(|envelope| envelope.compact_prompt_line()),
        Some("- boundary_envelope raw_chars=16 schedule_intent=none attachment_refs=0 explicit_locators=1 active_task_reference=none session_binding=none language_hint=none safety_budget_hint=none".to_string())
    );
    assert_eq!(
        ctx.route_result
            .as_ref()
            .map(|route| route.gate_kind().as_str()),
        Some("execute")
    );
}

#[test]
fn prepared_ask_flow_omits_empty_memory_and_recent_context() {
    let mut prepared_flow = prepared_flow_with_context();
    prepared_flow.recent_execution_context = "   ".to_string();

    let ctx = build_agent_run_context_from_prepared_flow("raw user request", &prepared_flow);

    assert_eq!(ctx.cross_turn_recent_execution_context, None);
}

#[test]
fn agent_loop_default_context_demotes_post_route_clarify_to_loop_context() {
    let mut route = base_route();
    route.needs_clarify = true;
    route.clarify_question = "missing target".to_string();
    route.set_clarify_gate();
    route.route_reason = "post_route_locator_guard".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let loop_ctx = agent_loop_default_context(Some(ctx)).expect("loop context");
    let route = loop_ctx.route_result.expect("route context");

    assert!(!route.needs_clarify);
    assert!(route.clarify_question.is_empty());
    assert_eq!(route.gate_kind(), crate::RouteGateKind::Execute);
    assert!(route.route_reason.contains("agent_loop_default_entry"));
}

#[test]
fn agent_loop_decides_respond_without_normalizer_direct_answer() {
    let mut route = base_route();
    route.set_ask_mode(crate::AskMode::respond_trace());
    route.route_reason = "direct_answer_trace_inferred".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let loop_ctx = agent_loop_default_context(Some(ctx)).expect("loop context");
    let route = loop_ctx.route_result.expect("route context");

    assert_eq!(route.gate_kind(), crate::RouteGateKind::Execute);
    assert!(!route.needs_clarify);
    assert!(route.route_reason.contains("agent_loop_default_entry"));
}

fn claimed_task(task_id: &str) -> crate::ClaimedTask {
    crate::ClaimedTask {
        task_id: task_id.to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    }
}

fn temp_workspace_root(label: &str) -> std::path::PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "rustclaw_post_route_refinement_{label}_{}_{}",
        std::process::id(),
        nonce
    ));
    std::fs::create_dir_all(&root).expect("temp root");
    root
}

fn boundary_locator_post_route(
    reason_code: &'static str,
) -> crate::post_route_policy::PostRoutePolicyResult {
    let mut route = base_route();
    route.needs_clarify = true;
    route.clarify_question = "missing locator".to_string();
    route.set_clarify_gate();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.requires_content_evidence = true;
    crate::post_route_policy::PostRoutePolicyResult {
        execution_route_result: route,
        auto_locator_path: None,
        auto_locator_hint: None,
        auto_locator_resolved_direct: false,
        fuzzy_locator_suggestions: Vec::new(),
        missing_locator_for_path_scoped_content: reason_code
            == "post_route_missing_path_scoped_locator",
        clarify_reason_kind: crate::post_route_policy::ClarifyReasonKind::RouteReasonText,
        gate_record: crate::post_route_policy::PostRouteGateRecord::with_owner(
            "boundary_locator_gate",
            reason_code,
            crate::post_route_policy::PostRoutePolicyOutcome::BoundaryClarify,
        ),
    }
}

#[test]
fn post_route_auto_locator_unbound_workspace_child_defers_to_agent_loop_candidate() {
    let root = temp_workspace_root("unbound_workspace_child");
    let readme = root.join("README.md");
    std::fs::write(&readme, "# Demo\n").expect("readme");
    let readme = readme.canonicalize().expect("canonical readme");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.clone();
    state.skill_rt.default_locator_search_dir = root.clone();
    let task = claimed_task("unbound-workspace-child-auto-locator");
    let session_snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let mut route = base_route();
    route.set_act_finalize(crate::ActFinalizeStyle::ChatWrapped);
    route.route_reason =
        "current_turn_anchor_overrides_contextual_target; executable_contract_preserved_for_agent_loop"
            .to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "README".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    let mut post_route = crate::post_route_policy::PostRoutePolicyResult {
        execution_route_result: route,
        auto_locator_path: Some(readme.display().to_string()),
        auto_locator_hint: None,
        auto_locator_resolved_direct: true,
        fuzzy_locator_suggestions: Vec::new(),
        missing_locator_for_path_scoped_content: false,
        clarify_reason_kind: crate::post_route_policy::ClarifyReasonKind::RouteReasonText,
        gate_record: crate::post_route_policy::PostRouteGateRecord::with_owner(
            "boundary_locator_gate",
            "post_route_auto_locator_satisfied_path_scoped_content",
            crate::post_route_policy::PostRoutePolicyOutcome::BoundaryReady,
        ),
    };
    let mut candidates = Vec::new();

    apply_post_route_refinements(
        &state,
        &task,
        "读一下那个 README 开头并用一句话总结",
        None,
        &session_snapshot,
        &mut candidates,
        &mut post_route,
    );

    assert_eq!(
        candidates.as_slice(),
        ["auto_locator_unbound_workspace_child_without_current_locator"]
    );
    assert!(post_route.auto_locator_path.is_none());
    assert!(!post_route.auto_locator_resolved_direct);
    assert!(post_route.missing_locator_for_path_scoped_content);
    assert_eq!(
        post_route.gate_record.reason_code,
        "post_route_auto_locator_unbound_workspace_child_deferred_to_agent_loop"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn boundary_observation_redacts_unbound_workspace_child_locator_path() {
    let root = temp_workspace_root("redact_unbound_workspace_child");
    let readme = root.join("README.md");
    std::fs::write(&readme, "# Demo\n").expect("readme");
    let readme = readme.canonicalize().expect("canonical readme");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.clone();
    state.skill_rt.default_locator_search_dir = root.clone();
    let session_snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let mut route = base_route();
    route.set_act_finalize(crate::ActFinalizeStyle::ChatWrapped);
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.requires_content_evidence = true;
    let post_route = crate::post_route_policy::PostRoutePolicyResult {
        execution_route_result: route,
        auto_locator_path: None,
        auto_locator_hint: None,
        auto_locator_resolved_direct: false,
        fuzzy_locator_suggestions: Vec::new(),
        missing_locator_for_path_scoped_content: true,
        clarify_reason_kind: crate::post_route_policy::ClarifyReasonKind::RouteReasonText,
        gate_record: crate::post_route_policy::PostRouteGateRecord::new(
            "post_route_auto_locator_unbound_workspace_child_deferred_to_agent_loop",
            crate::post_route_policy::PostRoutePolicyOutcome::RefineContract,
        ),
    };

    let block = agent_loop_boundary_observations_block(
        &state,
        &post_route,
        &session_snapshot,
        None,
        "读一下那个 README 开头并用一句话总结",
        "读一下那个 README 开头并用一句话总结",
        &["unbound_targeted_evidence"],
    )
    .expect("observation block");

    assert!(block.contains("\"resolved_workspace_child\":\"\""));
    assert!(block.contains("\"resolved_workspace_child_redacted\":true"));
    assert!(!block.contains(&readme.display().to_string()));
    assert!(block.contains("unbound_targeted_evidence"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn boundary_observation_redacts_non_boundary_clarify_workspace_child_path() {
    let root = temp_workspace_root("redact_non_boundary_clarify");
    let readme = root.join("README.md");
    std::fs::write(&readme, "# Demo\n").expect("readme");
    let readme = readme.canonicalize().expect("canonical readme");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.clone();
    state.skill_rt.default_locator_search_dir = root.clone();
    let mut route = base_route();
    route.route_reason = "standalone_freeform_clarify_loop_context".to_string();
    let post_route = crate::post_route_policy::PostRoutePolicyResult {
        execution_route_result: route,
        auto_locator_path: None,
        auto_locator_hint: None,
        auto_locator_resolved_direct: false,
        fuzzy_locator_suggestions: Vec::new(),
        missing_locator_for_path_scoped_content: false,
        clarify_reason_kind: crate::post_route_policy::ClarifyReasonKind::RouteReasonText,
        gate_record: crate::post_route_policy::PostRouteGateRecord::with_owner(
            "agent_loop_boundary_defer",
            "post_route_non_boundary_clarify_deferred_to_agent_loop",
            crate::post_route_policy::PostRoutePolicyOutcome::BoundaryReady,
        ),
    };

    let block = agent_loop_boundary_observations_block(
        &state,
        &post_route,
        &crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        },
        None,
        "读一下那个 README 开头并用一句话总结",
        "读一下那个 README 开头并用一句话总结",
        &[],
    )
    .expect("observation block");

    assert!(block.contains("\"resolved_workspace_child\":\"\""));
    assert!(block.contains("\"resolved_workspace_child_redacted\":true"));
    assert!(!block.contains(&readme.display().to_string()));
    assert!(block.contains("post_route_non_boundary_clarify_deferred_to_agent_loop"));
    assert!(block.contains("\"missing_referent\""));
    assert!(block.contains("\"reason_code\":\"unbound_deictic_reference\""));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn boundary_observation_hides_active_plan_files_for_unbound_referent() {
    let root = temp_workspace_root("unbound_referent_plan_files");
    let plan_dir = root.join("plan");
    std::fs::create_dir_all(&plan_dir).expect("plan dir");
    let active_plan = plan_dir.join("active.md");
    std::fs::write(&active_plan, "# Active Plan\n").expect("active plan");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.clone();
    state.skill_rt.default_locator_search_dir = root.clone();
    let mut route = base_route();
    route.route_reason = "standalone_freeform_clarify_loop_context".to_string();
    let post_route = crate::post_route_policy::PostRoutePolicyResult {
        execution_route_result: route,
        auto_locator_path: None,
        auto_locator_hint: None,
        auto_locator_resolved_direct: false,
        fuzzy_locator_suggestions: Vec::new(),
        missing_locator_for_path_scoped_content: false,
        clarify_reason_kind: crate::post_route_policy::ClarifyReasonKind::RouteReasonText,
        gate_record: crate::post_route_policy::PostRouteGateRecord::with_owner(
            "agent_loop_boundary_defer",
            "post_route_non_boundary_clarify_deferred_to_agent_loop",
            crate::post_route_policy::PostRoutePolicyOutcome::BoundaryReady,
        ),
    };

    let block = agent_loop_boundary_observations_block(
        &state,
        &post_route,
        &crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        },
        None,
        "summarize it",
        "summarize it",
        &[],
    )
    .expect("observation block");

    assert!(block.contains("\"missing_referent\""));
    assert!(block.contains("\"active_plan_files\":[]"));
    assert!(!block.contains(&active_plan.display().to_string()));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn boundary_observation_marks_deictic_filename_followup_without_active_target() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let mut route = base_route();
    route.route_reason = "execution_recipe_target_locator_preserved_for_agent_loop; executable_contract_preserved_for_agent_loop; current_turn_locator_overrides_contextual_path"
        .to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    let post_route = crate::post_route_policy::PostRoutePolicyResult {
        execution_route_result: route,
        auto_locator_path: None,
        auto_locator_hint: None,
        auto_locator_resolved_direct: false,
        fuzzy_locator_suggestions: Vec::new(),
        missing_locator_for_path_scoped_content: false,
        clarify_reason_kind: crate::post_route_policy::ClarifyReasonKind::RouteReasonText,
        gate_record: crate::post_route_policy::PostRouteGateRecord::new(
            "post_route_no_change",
            crate::post_route_policy::PostRoutePolicyOutcome::NoChange,
        ),
    };

    let block = agent_loop_boundary_observations_block(
        &state,
        &post_route,
        &crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        },
        None,
        "continue that project and update test_calc_core.py",
        "continue that project and update test_calc_core.py",
        &[],
    )
    .expect("deictic filename followup without target should export boundary state");

    assert!(block.contains("\"missing_referent\""));
    assert!(block.contains("\"reason_code\":\"unbound_deictic_reference\""));
    assert!(block.contains("\"source\":\"unbound_contextual_locator\""));
    assert!(block.contains("\"active_bound_targets\":[]"));
}

#[test]
fn boundary_observation_suppresses_missing_referent_after_auto_locator_ready() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let mut route = base_route();
    route.route_reason = "execution_recipe_target_locator_preserved_for_agent_loop; executable_contract_preserved_for_agent_loop; current_turn_locator_overrides_contextual_path"
        .to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Filename;
    route.output_contract.requires_content_evidence = true;
    let post_route = crate::post_route_policy::PostRoutePolicyResult {
        execution_route_result: route,
        auto_locator_path: Some("/workspace/rustclaw.service".to_string()),
        auto_locator_hint: None,
        auto_locator_resolved_direct: true,
        fuzzy_locator_suggestions: Vec::new(),
        missing_locator_for_path_scoped_content: false,
        clarify_reason_kind: crate::post_route_policy::ClarifyReasonKind::RouteReasonText,
        gate_record: crate::post_route_policy::PostRouteGateRecord::with_owner(
            "boundary_locator_gate",
            "post_route_auto_locator_satisfied_path_scoped_content",
            crate::post_route_policy::PostRoutePolicyOutcome::BoundaryReady,
        ),
    };

    let block = agent_loop_boundary_observations_block(
        &state,
        &post_route,
        &crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        },
        None,
        "rustclaw.service",
        "rustclaw.service",
        &[],
    )
    .expect("auto locator boundary should export boundary state");

    assert!(block.contains("\"auto_locator\""));
    assert!(block.contains("\"resolved_direct\":true"));
    assert!(block.contains("\"outcome\":\"boundary_ready\""));
    assert!(block.contains("\"missing_referent\":null"));
}

#[test]
fn boundary_observation_does_not_export_active_plan_files_for_plain_answer_boundary() {
    let root = temp_workspace_root("plain_answer_hides_plan_files");
    let plan_dir = root.join("plan");
    std::fs::create_dir_all(&plan_dir).expect("plan dir");
    let active_plan = plan_dir.join("active.md");
    std::fs::write(&active_plan, "# Active Plan\n").expect("active plan");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.clone();
    state.skill_rt.default_locator_search_dir = root.clone();
    let mut route = base_route();
    route.route_reason.clear();
    let post_route = crate::post_route_policy::PostRoutePolicyResult {
        execution_route_result: route,
        auto_locator_path: None,
        auto_locator_hint: None,
        auto_locator_resolved_direct: false,
        fuzzy_locator_suggestions: Vec::new(),
        missing_locator_for_path_scoped_content: false,
        clarify_reason_kind: crate::post_route_policy::ClarifyReasonKind::RouteReasonText,
        gate_record: crate::post_route_policy::PostRouteGateRecord::new(
            "post_route_no_change",
            crate::post_route_policy::PostRoutePolicyOutcome::NoChange,
        ),
    };

    let block = agent_loop_boundary_observations_block(
        &state,
        &post_route,
        &crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        },
        None,
        "draft from current conversation",
        "draft from current conversation",
        &[],
    );

    assert!(block.is_none());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn boundary_observation_exports_active_plan_files_for_workspace_summary_contract() {
    let root = temp_workspace_root("workspace_summary_plan_files");
    let plan_dir = root.join("plan");
    std::fs::create_dir_all(&plan_dir).expect("plan dir");
    let active_plan = plan_dir.join("active.md");
    std::fs::write(&active_plan, "# Active Plan\n").expect("active plan");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.clone();
    state.skill_rt.default_locator_search_dir = root.clone();
    let mut route = base_route();
    route.route_reason = "workspace_project_summary".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::WorkspaceProjectSummary;
    let post_route = crate::post_route_policy::PostRoutePolicyResult {
        execution_route_result: route,
        auto_locator_path: None,
        auto_locator_hint: None,
        auto_locator_resolved_direct: false,
        fuzzy_locator_suggestions: Vec::new(),
        missing_locator_for_path_scoped_content: false,
        clarify_reason_kind: crate::post_route_policy::ClarifyReasonKind::RouteReasonText,
        gate_record: crate::post_route_policy::PostRouteGateRecord::new(
            "post_route_no_change",
            crate::post_route_policy::PostRoutePolicyOutcome::NoChange,
        ),
    };

    let block = agent_loop_boundary_observations_block(
        &state,
        &post_route,
        &crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        },
        None,
        "summarize workspace",
        "summarize workspace",
        &[],
    )
    .expect("workspace summary should export boundary observation");

    assert!(block.contains("\"active_plan_files\""));
    assert!(block.contains(&active_plan.display().to_string()));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn post_route_missing_locator_boundary_defers_to_agent_loop_candidate() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let task = claimed_task("missing-locator-boundary-defer");
    let session_snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let mut post_route = boundary_locator_post_route("post_route_missing_path_scoped_locator");
    let mut candidates = Vec::new();

    apply_post_route_refinements(
        &state,
        &task,
        "inspect project file",
        None,
        &session_snapshot,
        &mut candidates,
        &mut post_route,
    );

    assert!(!post_route.execution_route_result.needs_clarify);
    assert!(post_route
        .execution_route_result
        .clarify_question
        .is_empty());
    assert_eq!(
        candidates.as_slice(),
        ["post_route_missing_path_scoped_locator"]
    );
    assert_eq!(
        post_route.gate_record.owner_layer,
        "agent_loop_boundary_defer"
    );
    assert_eq!(
        post_route.gate_record.reason_code,
        "post_route_missing_path_scoped_locator_deferred_to_agent_loop"
    );
}

#[test]
fn agent_loop_decides_clarify_without_post_route_force_clarify() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let task = claimed_task("loop-owned-clarify-boundary");
    let session_snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let mut post_route = boundary_locator_post_route("post_route_missing_path_scoped_locator");
    let mut candidates = Vec::new();

    apply_post_route_refinements(
        &state,
        &task,
        "inspect project file",
        None,
        &session_snapshot,
        &mut candidates,
        &mut post_route,
    );

    assert!(!post_route.execution_route_result.needs_clarify);
    assert_eq!(
        post_route.execution_route_result.gate_kind(),
        crate::RouteGateKind::Execute
    );
    assert_eq!(candidates, vec!["post_route_missing_path_scoped_locator"]);
}

#[test]
fn post_route_fuzzy_locator_boundary_defers_to_agent_loop_candidate() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let task = claimed_task("fuzzy-locator-boundary-defer");
    let session_snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let mut post_route = boundary_locator_post_route("post_route_fuzzy_locator_candidates");
    post_route.fuzzy_locator_suggestions =
        vec!["README.md".to_string(), "README.zh-CN.md".to_string()];
    let mut candidates = Vec::new();

    apply_post_route_refinements(
        &state,
        &task,
        "inspect readme",
        None,
        &session_snapshot,
        &mut candidates,
        &mut post_route,
    );

    assert!(!post_route.execution_route_result.needs_clarify);
    assert_eq!(
        candidates.as_slice(),
        ["post_route_fuzzy_locator_candidates"]
    );
    assert_eq!(
        post_route.gate_record.reason_code,
        "post_route_fuzzy_locator_candidates_deferred_to_agent_loop"
    );
}

#[test]
fn post_route_non_locator_boundary_clarify_stays_boundary_owned() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let task = claimed_task("non-locator-boundary-stays");
    let session_snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let mut post_route = boundary_locator_post_route("post_route_boundary_clarify_required");
    post_route.gate_record = crate::post_route_policy::PostRouteGateRecord::with_owner(
        "boundary_clarify_gate",
        "post_route_boundary_clarify_required",
        crate::post_route_policy::PostRoutePolicyOutcome::BoundaryClarify,
    );
    let mut candidates = Vec::new();

    apply_post_route_refinements(
        &state,
        &task,
        "send workspace root as a file",
        None,
        &session_snapshot,
        &mut candidates,
        &mut post_route,
    );

    assert!(post_route.execution_route_result.needs_clarify);
    assert!(candidates.is_empty());
    assert_eq!(
        post_route.gate_record.reason_code,
        "post_route_boundary_clarify_required"
    );
}

#[test]
fn boundary_observation_block_filters_natural_language_route_reason() {
    let mut route = base_route();
    route.needs_clarify = true;
    route.set_clarify_gate();
    route.route_reason =
        "用户自然语言原因; machine_boundary_code; MixedCaseCode; another.machine_code".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.requires_content_evidence = true;

    let post_route = crate::post_route_policy::PostRoutePolicyResult {
        execution_route_result: route,
        auto_locator_path: Some("/tmp/workspace/README.md".to_string()),
        auto_locator_hint: None,
        auto_locator_resolved_direct: true,
        fuzzy_locator_suggestions: Vec::new(),
        missing_locator_for_path_scoped_content: false,
        clarify_reason_kind: crate::post_route_policy::ClarifyReasonKind::RouteReasonText,
        gate_record: crate::post_route_policy::PostRouteGateRecord::new(
            "post_route_locator_boundary",
            crate::post_route_policy::PostRoutePolicyOutcome::BoundaryClarify,
        ),
    };
    let session_snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            alias_bindings: vec![crate::conversation_state::SessionAliasBinding {
                alias: "docs alias".to_string(),
                target: "/tmp/workspace/docs".to_string(),
                updated_at_ts: 1,
            }],
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: Some(crate::observed_facts::ObservedFacts {
            bound_target: Some("/tmp/workspace/notes.md".to_string()),
            ordered_entries: vec!["one.md".to_string(), "two.md".to_string()],
            observed_entry_count: Some(2),
            ..crate::observed_facts::ObservedFacts::default()
        }),
    };

    let state = crate::AppState::test_default_with_fixture_provider();
    let block = agent_loop_boundary_observations_block(
        &state,
        &post_route,
        &session_snapshot,
        None,
        "inspect /tmp/workspace/README.md",
        "resolved capability_ref=kb.list_namespaces",
        &[],
    )
    .expect("observation block");

    assert!(block.contains("AGENT_LOOP_BOUNDARY_OBSERVATIONS"));
    assert!(block.contains("machine_boundary_code"));
    assert!(block.contains("another.machine_code"));
    assert!(block.contains("/tmp/workspace/README.md"));
    assert!(block.contains("current_request_locator"));
    assert!(block.contains("explicit_locator_hints"));
    assert!(block.contains("registry_capability_contract"));
    assert!(block.contains("kb.list_namespaces"));
    assert!(block.contains("post_route_boundary_record"));
    assert!(!block.contains("\"post_route_policy\""));
    assert!(block.contains("docs alias"));
    assert!(block.contains("/tmp/workspace/docs"));
    assert!(block.contains("active_observed_facts"));
    assert!(block.contains("/tmp/workspace/notes.md"));
    assert!(!block.contains("route_gate_kind"));
    assert!(!block.contains("用户自然语言原因"));
    assert!(!block.contains("MixedCaseCode"));
}

#[test]
fn boundary_observation_block_omits_legacy_route_trace_only_reason() {
    let mut route = base_route();
    route.route_reason = "executionless_finalize_trace_plain".to_string();
    let post_route = crate::post_route_policy::PostRoutePolicyResult {
        execution_route_result: route,
        auto_locator_path: None,
        auto_locator_hint: None,
        auto_locator_resolved_direct: false,
        fuzzy_locator_suggestions: Vec::new(),
        missing_locator_for_path_scoped_content: false,
        clarify_reason_kind: crate::post_route_policy::ClarifyReasonKind::RouteReasonText,
        gate_record: crate::post_route_policy::PostRouteGateRecord::new(
            "post_route_no_change",
            crate::post_route_policy::PostRoutePolicyOutcome::NoChange,
        ),
    };
    let state = crate::AppState::test_default_with_fixture_provider();

    let block = agent_loop_boundary_observations_block(
        &state,
        &post_route,
        &crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        },
        None,
        "answer from current runtime context",
        "answer from current runtime context",
        &[],
    );

    assert!(block.is_none());
}

#[test]
fn boundary_observation_block_exports_runtime_session_state_for_status_query() {
    let route = base_route();
    let post_route = crate::post_route_policy::PostRoutePolicyResult {
        execution_route_result: route,
        auto_locator_path: None,
        auto_locator_hint: None,
        auto_locator_resolved_direct: false,
        fuzzy_locator_suggestions: Vec::new(),
        missing_locator_for_path_scoped_content: false,
        clarify_reason_kind: crate::post_route_policy::ClarifyReasonKind::RouteReasonText,
        gate_record: crate::post_route_policy::PostRouteGateRecord::new(
            "post_route_no_change",
            crate::post_route_policy::PostRoutePolicyOutcome::NoChange,
        ),
    };
    let state = crate::AppState::test_default_with_fixture_provider();
    let turn_analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::StatusQuery),
        target_task_policy: None,
        should_interrupt_active_run: false,
        state_patch: Some(serde_json::json!({
            "runtime_status_query": {"kind": "current_user_boundary", "scope": "session"}
        })),
        attachment_processing_required: false,
    };

    let block = agent_loop_boundary_observations_block(
        &state,
        &post_route,
        &crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        },
        Some(&turn_analysis),
        "answer from current runtime context",
        "answer from current runtime context",
        &[],
    )
    .expect("runtime status query should export boundary state");

    assert!(block.contains("\"runtime_session_state\""));
    assert!(block.contains("\"active_task_present\":false"));
    assert!(block.contains("\"pending_user_boundary_present\":false"));
}

#[test]
fn boundary_observation_exports_code_workspace_followup_anchor() {
    let route = base_route();
    let post_route = crate::post_route_policy::PostRoutePolicyResult {
        execution_route_result: route,
        auto_locator_path: None,
        auto_locator_hint: None,
        auto_locator_resolved_direct: false,
        fuzzy_locator_suggestions: Vec::new(),
        missing_locator_for_path_scoped_content: false,
        clarify_reason_kind: crate::post_route_policy::ClarifyReasonKind::RouteReasonText,
        gate_record: crate::post_route_policy::PostRouteGateRecord::new(
            "post_route_no_change",
            crate::post_route_policy::PostRoutePolicyOutcome::NoChange,
        ),
    };
    let state = crate::AppState::test_default_with_fixture_provider();
    let project_dir = "/home/guagua/rustclaw/run/nl_eval_tmp/code_workspace_anchor";
    let frame = crate::followup_frame::FollowupFrame {
        source_request: "code workspace update".to_string(),
        op_kind: crate::followup_frame::FollowupOpKind::CodeWorkspace,
        bound_target: Some(project_dir.to_string()),
        source_task_id: "task-code-workspace".to_string(),
        ..crate::followup_frame::FollowupFrame::default()
    };

    let block = agent_loop_boundary_observations_block(
        &state,
        &post_route,
        &crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: Some(frame),
            active_clarify_state: None,
            active_observed_facts: None,
        },
        None,
        "code workspace followup",
        "code workspace followup",
        &[],
    )
    .expect("code workspace anchor should export boundary state");

    assert!(block.contains("\"active_bound_targets\""));
    assert!(block.contains("\"op_kind\":\"code_workspace\""));
    assert!(block.contains(project_dir));
}

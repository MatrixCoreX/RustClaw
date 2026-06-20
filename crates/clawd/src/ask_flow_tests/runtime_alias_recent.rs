#[test]
fn normalizer_runtime_fact_direct_answer_skips_execute_gate() {
    let Some(runtime_user) = ["USER", "LOGNAME", "USERNAME"]
        .into_iter()
        .filter_map(|key| std::env::var(key).ok())
        .map(|value| value.trim().to_string())
        .find(|value| !value.is_empty())
    else {
        return;
    };
    let state = crate::AppState::test_default_with_fixture_provider();
    let mut route = chat_route_for_gate();
    route.ask_mode = crate::AskMode::planner_execute_plain();
    route.resolved_intent = format!("runtime_scalar\nanswer_candidate: {runtime_user}");
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = false;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert!(normalizer_runtime_fact_direct_answer_candidate(
        &state,
        &format!("runtime_scalar\nanswer_candidate: {runtime_user}"),
        Some(&ctx),
    )
    .is_none());
}

#[test]
fn runtime_scalar_path_direct_answer_uses_verified_contract_locator() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let runtime_path = state.skill_rt.workspace_root.to_string_lossy().to_string();
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "Output the current workspace path".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "runtime scalar path".to_string(),
        route_confidence: Some(1.0),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Scalar,
            semantic_kind: crate::OutputSemanticKind::ScalarPathOnly,
            locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
            locator_hint: runtime_path.clone(),
            ..Default::default()
        },
    };
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert_eq!(
        runtime_scalar_path_direct_answer_candidate(&state, Some(&ctx)).as_deref(),
        Some(runtime_path.as_str())
    );
}

#[test]
fn runtime_scalar_path_direct_answer_uses_verified_workspace_path_locator() {
    let root = TempDirGuard::new("runtime_scalar_path_workspace_locator");
    let docs = root.path.join("docs");
    std::fs::create_dir_all(&docs).expect("docs dir");
    let target = docs.join("release_checklist.md");
    std::fs::write(&target, "- item\n").expect("write target");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.path.clone();
    state.skill_rt.default_locator_search_dir = root.path.clone();
    let target = target.display().to_string();
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "Return the bound path only".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "runtime scalar bound path".to_string(),
        route_confidence: Some(1.0),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Free,
            semantic_kind: crate::OutputSemanticKind::ScalarPathOnly,
            locator_kind: crate::OutputLocatorKind::Path,
            locator_hint: target.clone(),
            requires_content_evidence: false,
            delivery_required: false,
            delivery_intent: crate::OutputDeliveryIntent::None,
            ..Default::default()
        },
    };
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert_eq!(
        runtime_scalar_path_direct_answer_candidate(&state, Some(&ctx)).as_deref(),
        Some(target.as_str())
    );
}

#[test]
fn runtime_scalar_path_direct_answer_keeps_evidence_required_paths_in_planner() {
    let root = TempDirGuard::new("runtime_scalar_path_evidence_required");
    let target = root.path.join("release_checklist.md");
    std::fs::write(&target, "- item\n").expect("write target");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.path.clone();
    state.skill_rt.default_locator_search_dir = root.path.clone();
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "Return the path after observing it".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "runtime scalar bound path".to_string(),
        route_confidence: Some(1.0),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Scalar,
            semantic_kind: crate::OutputSemanticKind::ScalarPathOnly,
            locator_kind: crate::OutputLocatorKind::Path,
            locator_hint: target.display().to_string(),
            requires_content_evidence: true,
            delivery_required: false,
            delivery_intent: crate::OutputDeliveryIntent::None,
            ..Default::default()
        },
    };
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert_eq!(
        runtime_scalar_path_direct_answer_candidate(&state, Some(&ctx)),
        None
    );
}

#[test]
fn runtime_scalar_path_direct_answer_rejects_unverified_locator() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "Output the current workspace path".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "runtime scalar path".to_string(),
        route_confidence: Some(1.0),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Scalar,
            semantic_kind: crate::OutputSemanticKind::ScalarPathOnly,
            locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
            locator_hint: "/tmp/not-the-rustclaw-workspace".to_string(),
            ..Default::default()
        },
    };
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert_eq!(
        runtime_scalar_path_direct_answer_candidate(&state, Some(&ctx)),
        None
    );
}

#[test]
fn active_file_basename_direct_answer_uses_active_bound_file_target() {
    let root = TempDirGuard::new("active_file_basename");
    let target = root.path.join("README.md");
    std::fs::write(&target, "# Demo\n").expect("write readme");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.path.clone();
    state.skill_rt.default_locator_search_dir = root.path.clone();
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::direct_answer(),
        resolved_intent: "Return the active file target basename.".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "active file basename contract".to_string(),
        route_confidence: Some(1.0),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Scalar,
            semantic_kind: crate::OutputSemanticKind::FileBasename,
            locator_kind: crate::OutputLocatorKind::None,
            locator_hint: String::new(),
            ..Default::default()
        },
    };
    let target = target.display().to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        context_bundle_summary: Some(format!(
            "route_view=false\n\n\
### ACTIVE_EXECUTION_ANCHOR\n\
followup_source_request: read opening\n\
followup_op_kind: Read\n\
followup_bound_target: {target}\n\
observed_bound_target: {target}\n"
        )),
        ..Default::default()
    };

    assert_eq!(
        active_file_basename_direct_answer_candidate(&state, Some(&ctx)).as_deref(),
        Some("README.md")
    );
}

#[test]
fn active_file_basename_direct_answer_accepts_empty_filename_locator_contract() {
    let root = TempDirGuard::new("active_file_basename_filename_locator");
    let target = root.path.join("README.md");
    std::fs::write(&target, "# Demo\n").expect("write readme");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.path.clone();
    state.skill_rt.default_locator_search_dir = root.path.clone();
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "Return the active file target basename.".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "file_basename contract from active execution anchor".to_string(),
        route_confidence: Some(1.0),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Scalar,
            semantic_kind: crate::OutputSemanticKind::FileBasename,
            locator_kind: crate::OutputLocatorKind::Filename,
            locator_hint: String::new(),
            ..Default::default()
        },
    };
    let target = target.display().to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        context_bundle_summary: Some(format!(
            "route_view=false execution_view=true\n\n\
### ACTIVE_EXECUTION_ANCHOR\n\
followup_source_request: deliver file\n\
followup_op_kind: Delivery\n\
followup_bound_target: {target}\n\
observed_bound_target: {target}\n\
observed_ordered_entries: 1:{target}\n"
        )),
        ..Default::default()
    };

    assert_eq!(
        active_file_basename_direct_answer_candidate(&state, Some(&ctx)).as_deref(),
        Some("README.md")
    );
}

#[test]
fn active_file_basename_direct_answer_accepts_matching_filename_locator_hint() {
    let root = TempDirGuard::new("active_file_basename_filename_hint");
    let target = root.path.join("README.md");
    std::fs::write(&target, "# Demo\n").expect("write readme");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.path.clone();
    state.skill_rt.default_locator_search_dir = root.path.clone();
    let target = target.display().to_string();
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "Return the active file target basename.".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "file_basename contract from active execution anchor".to_string(),
        route_confidence: Some(1.0),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Scalar,
            semantic_kind: crate::OutputSemanticKind::FileBasename,
            locator_kind: crate::OutputLocatorKind::Filename,
            locator_hint: target.clone(),
            ..Default::default()
        },
    };
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        context_bundle_summary: Some(format!(
            "route_view=false execution_view=true\n\n\
### ACTIVE_EXECUTION_ANCHOR\n\
followup_source_request: deliver file\n\
followup_op_kind: Delivery\n\
followup_bound_target: {target}\n\
observed_bound_target: {target}\n\
observed_ordered_entries: 1:{target}\n"
        )),
        ..Default::default()
    };

    assert_eq!(
        active_file_basename_direct_answer_candidate(&state, Some(&ctx)).as_deref(),
        Some("README.md")
    );
}

#[test]
fn active_file_basename_direct_answer_uses_recent_delivery_token() {
    let root = TempDirGuard::new("active_file_basename_recent_delivery");
    let target = root.path.join("clawd-dev.log");
    std::fs::write(&target, "line\n").expect("write log");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.path.clone();
    state.skill_rt.default_locator_search_dir = root.path.clone();
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "Return the basename of the recently delivered file.".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "direct_answer_decision_overridden_by_executable_contract".to_string(),
        route_confidence: Some(1.0),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Scalar,
            semantic_kind: crate::OutputSemanticKind::FileBasename,
            locator_kind: crate::OutputLocatorKind::None,
            locator_hint: String::new(),
            ..Default::default()
        },
    };
    let target = target.display().to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        context_bundle_summary: Some("route_view=false execution_view=true".to_string()),
        cross_turn_recent_execution_context: Some(format!(
            "### RECENT_EXECUTION_ANCHOR\n- latest_result=FILE:{target}\n"
        )),
        ..Default::default()
    };

    assert_eq!(
        active_file_basename_direct_answer_candidate(&state, Some(&ctx)).as_deref(),
        Some("clawd-dev.log")
    );
}

#[test]
fn active_file_basename_direct_answer_overrides_clarify_evidence_misroute() {
    let root = TempDirGuard::new("active_file_basename_clarify");
    let target = root.path.join("README.txt");
    std::fs::write(&target, "# Demo\n").expect("write readme");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.path.clone();
    state.skill_rt.default_locator_search_dir = root.path.clone();
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::clarify(),
        resolved_intent: "Return the active file target basename.".to_string(),
        needs_clarify: true,
        clarify_question: String::new(),
        route_reason: "active file basename contract needs evidence".to_string(),
        route_confidence: Some(1.0),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Scalar,
            semantic_kind: crate::OutputSemanticKind::FileBasename,
            locator_kind: crate::OutputLocatorKind::None,
            locator_hint: String::new(),
            requires_content_evidence: true,
            ..Default::default()
        },
    };
    let target = target.display().to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        context_bundle_summary: Some(format!(
            "route_view=false\n\n\
### ACTIVE_EXECUTION_ANCHOR\n\
followup_source_request: deliver first entry\n\
followup_op_kind: Delivery\n\
followup_bound_target: {target}\n\
observed_bound_target: {target}\n"
        )),
        ..Default::default()
    };

    assert_eq!(
        active_file_basename_direct_answer_candidate(&state, Some(&ctx)).as_deref(),
        Some("README.txt")
    );
}

#[test]
fn active_file_basename_direct_answer_uses_bound_answer_candidate_for_generic_scalar_contract() {
    let root = TempDirGuard::new("active_file_basename_candidate");
    let target = root.path.join("README.txt");
    std::fs::write(&target, "# Demo\n").expect("write readme");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.path.clone();
    state.skill_rt.default_locator_search_dir = root.path.clone();
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "Return the compact active result.\nanswer_candidate: README.txt"
            .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "generic scalar candidate from active file result".to_string(),
        route_confidence: Some(1.0),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Scalar,
            semantic_kind: crate::OutputSemanticKind::None,
            locator_kind: crate::OutputLocatorKind::None,
            locator_hint: String::new(),
            requires_content_evidence: true,
            ..Default::default()
        },
    };
    let target = target.display().to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        context_bundle_summary: Some(format!(
            "route_view=false\n\n\
### ACTIVE_EXECUTION_ANCHOR\n\
followup_source_request: deliver first entry\n\
followup_op_kind: Delivery\n\
followup_bound_target: {target}\n\
observed_bound_target: {target}\n"
        )),
        ..Default::default()
    };

    assert_eq!(
        active_file_basename_direct_answer_candidate(&state, Some(&ctx)).as_deref(),
        Some("README.txt")
    );
}

#[test]
fn active_file_basename_direct_answer_uses_context_answer_candidate() {
    let root = TempDirGuard::new("active_file_basename_context_candidate");
    let target = root.path.join("release_checklist.md");
    std::fs::write(&target, "# Demo\n").expect("write release");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.path.clone();
    state.skill_rt.default_locator_search_dir = root.path.clone();
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "Return compact active result.".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "direct_answer_decision_overridden_by_executable_contract".to_string(),
        route_confidence: Some(1.0),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Scalar,
            semantic_kind: crate::OutputSemanticKind::None,
            locator_kind: crate::OutputLocatorKind::None,
            locator_hint: String::new(),
            requires_content_evidence: true,
            ..Default::default()
        },
    };
    let target = target.display().to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        context_bundle_summary: Some(format!(
            "route_view=false resolved_prompt=only basename\n\
answer_candidate: release_checklist.md\n\n\
### ACTIVE_EXECUTION_ANCHOR\n\
followup_source_request: deliver release checklist\n\
followup_op_kind: Delivery\n\
followup_bound_target: {target}\n\
observed_bound_target: {target}\n"
        )),
        ..Default::default()
    };

    assert_eq!(
        active_file_basename_direct_answer_candidate(&state, Some(&ctx)).as_deref(),
        Some("release_checklist.md")
    );
}

#[test]
fn active_file_basename_direct_answer_observation_satisfies_field_value_evidence() {
    let root = TempDirGuard::new("active_file_basename_evidence");
    let target = root.path.join("clawd-codex-current.log");
    std::fs::write(&target, "tail\n").expect("write log");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.path.clone();
    state.skill_rt.default_locator_search_dir = root.path.clone();
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "Return only the active delivered file basename.".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "direct_answer_decision_overridden_by_executable_contract".to_string(),
        route_confidence: Some(1.0),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Scalar,
            semantic_kind: crate::OutputSemanticKind::FileBasename,
            locator_kind: crate::OutputLocatorKind::None,
            locator_hint: String::new(),
            requires_content_evidence: false,
            ..Default::default()
        },
    };
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route.clone()),
        context_bundle_summary: Some(format!(
            "route_view=false\n\n\
### ACTIVE_EXECUTION_ANCHOR\n\
followup_source_request: deliver second entry\n\
followup_op_kind: Delivery\n\
followup_bound_target: {}\n\
observed_bound_target: {}\n",
            target.display(),
            target.display()
        )),
        ..Default::default()
    };

    let candidate = active_file_basename_direct_answer(&state, Some(&ctx)).expect("candidate");
    assert_eq!(candidate.answer, "clawd-codex-current.log");
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-active-basename",
        "ask",
        "只说这个文件名",
    );
    journal.record_route_result(&route);
    journal.push_task_observation(candidate.observed_evidence());
    let coverage = crate::task_journal::evidence_coverage_for_route(&route, &journal);

    assert!(coverage.observed_canonical.contains("field_value"));
    assert!(coverage.missing_evidence.is_empty(), "{coverage:?}");
}

#[test]
fn active_file_basename_direct_answer_uses_bound_answer_candidate_for_scalar_path_contract() {
    let root = TempDirGuard::new("active_file_basename_scalar_path");
    let target = root.path.join("README.txt");
    std::fs::write(&target, "# Demo\n").expect("write readme");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.path.clone();
    state.skill_rt.default_locator_search_dir = root.path.clone();
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent:
            "Return the basename of the active delivered file.\nanswer_candidate: README.txt"
                .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "scalar path candidate from active file result".to_string(),
        route_confidence: Some(1.0),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Scalar,
            semantic_kind: crate::OutputSemanticKind::ScalarPathOnly,
            locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
            locator_hint: String::new(),
            requires_content_evidence: true,
            ..Default::default()
        },
    };
    let target = target.display().to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        context_bundle_summary: Some(format!(
            "route_view=false\n\n\
### ACTIVE_EXECUTION_ANCHOR\n\
followup_source_request: deliver first entry\n\
followup_op_kind: Delivery\n\
followup_bound_target: {target}\n\
observed_bound_target: {target}\n"
        )),
        ..Default::default()
    };

    assert_eq!(
        active_file_basename_direct_answer_candidate(&state, Some(&ctx)).as_deref(),
        Some("README.txt")
    );
}

#[test]
fn active_file_basename_direct_answer_uses_delivery_anchor_for_file_names_contract() {
    let root = TempDirGuard::new("active_file_basename_file_names");
    let target = root.path.join("README.txt");
    std::fs::write(&target, "# Demo\n").expect("write readme");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.path.clone();
    state.skill_rt.default_locator_search_dir = root.path.clone();
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "Return the active delivered file name.".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "file names contract from active file delivery".to_string(),
        route_confidence: Some(1.0),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Strict,
            semantic_kind: crate::OutputSemanticKind::FileNames,
            locator_kind: crate::OutputLocatorKind::None,
            locator_hint: String::new(),
            requires_content_evidence: true,
            ..Default::default()
        },
    };
    let target = target.display().to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        context_bundle_summary: Some(format!(
            "route_view=false\n\n\
### ACTIVE_EXECUTION_ANCHOR\n\
followup_source_request: deliver first entry\n\
followup_op_kind: Delivery\n\
followup_bound_target: {target}\n\
observed_bound_target: {target}\n"
        )),
        ..Default::default()
    };

    assert_eq!(
        active_file_basename_direct_answer_candidate(&state, Some(&ctx)).as_deref(),
        Some("README.txt")
    );
}

#[test]
fn active_file_basename_direct_answer_uses_delivery_anchor_mutation_marker() {
    let root = TempDirGuard::new("active_file_basename_delivery_marker");
    let target = root.path.join("Report.MD");
    std::fs::write(&target, "# Demo\n").expect("write report");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.path.clone();
    state.skill_rt.default_locator_search_dir = root.path.clone();
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "Return a direct active delivery follow-up.".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "active_task_mutation_to_direct_answer".to_string(),
        route_confidence: Some(1.0),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Free,
            semantic_kind: crate::OutputSemanticKind::None,
            locator_kind: crate::OutputLocatorKind::None,
            locator_hint: String::new(),
            requires_content_evidence: true,
            ..Default::default()
        },
    };
    let target = target.display().to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        context_bundle_summary: Some(format!(
            "route_view=false\n\n\
### ACTIVE_EXECUTION_ANCHOR\n\
followup_source_request: deliver this file\n\
followup_op_kind: Delivery\n\
followup_bound_target: {target}\n\
observed_bound_target: {target}\n"
        )),
        ..Default::default()
    };

    assert_eq!(
        active_file_basename_direct_answer_candidate(&state, Some(&ctx)).as_deref(),
        Some("Report.MD")
    );
}

#[test]
fn active_file_basename_direct_answer_rejects_generic_scalar_contract() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let target = state.skill_rt.workspace_root.join("README.md");
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::direct_answer(),
        resolved_intent: "Return a scalar from the active output.".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "generic scalar contract".to_string(),
        route_confidence: Some(1.0),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Scalar,
            semantic_kind: crate::OutputSemanticKind::None,
            locator_kind: crate::OutputLocatorKind::None,
            locator_hint: String::new(),
            ..Default::default()
        },
    };
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        context_bundle_summary: Some(format!(
            "route_view=false\n\n\
### ACTIVE_EXECUTION_ANCHOR\n\
followup_bound_target: {}\n",
            target.display()
        )),
        ..Default::default()
    };

    assert_eq!(
        active_file_basename_direct_answer_candidate(&state, Some(&ctx)),
        None
    );
}

#[test]
fn preferred_route_clarify_question_respects_explicit_route_question_before_generic_fallback() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::clarify(),
        resolved_intent: "看看那个目录下面都有什么".to_string(),
        needs_clarify: true,
        clarify_question: "LOCATOR_CLARIFY_PROMPT".to_string(),
        route_reason: "fresh_deictic_missing_locator:directory_lookup".to_string(),
        route_confidence: Some(0.95),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Free,
            requires_content_evidence: true,
            locator_kind: crate::OutputLocatorKind::Path,
            ..Default::default()
        },
    };
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route.clone()),
        ..Default::default()
    };
    assert_eq!(
        preferred_route_clarify_question(Some(&ctx)).as_deref(),
        Some("LOCATOR_CLARIFY_PROMPT")
    );

    route.clarify_question.clear();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    assert_eq!(preferred_route_clarify_question(Some(&ctx)), None);
    let context = route_structured_clarify_context(Some(&ctx)).expect("structured context");
    assert!(context.contains("clarify_case: missing_read_target"));
    assert!(context.contains("locator_kind: path"));
}

#[test]
fn structured_clarify_context_uses_reason_code_for_raw_output_missing_read_target() {
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::clarify(),
        resolved_intent: "查看指定日志文件的最近20行内容".to_string(),
        needs_clarify: true,
        clarify_question: String::new(),
        route_reason:
            "semantic_contract_requires_evidence; clarify_reason_code:missing_read_target"
                .to_string(),
        route_confidence: Some(0.95),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Strict,
            requires_content_evidence: true,
            locator_kind: crate::OutputLocatorKind::None,
            semantic_kind: crate::OutputSemanticKind::RawCommandOutput,
            ..Default::default()
        },
    };
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let context = route_structured_clarify_context(Some(&ctx)).expect("structured context");
    assert!(context.contains("clarify_case: missing_read_target"));
    assert!(context.contains("response_shape: strict"));
    assert!(context.contains("semantic_kind: raw_command_output"));
}

#[test]
fn fuzzy_locator_candidates_are_structured_context_not_hard_question() {
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::clarify(),
        resolved_intent: "读取 Cargo.toml 的 package.name，只输出值".to_string(),
        needs_clarify: true,
        clarify_question: String::new(),
        route_reason: "llm_contract:generic_filename_scalar_extract".to_string(),
        route_confidence: Some(0.95),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Scalar,
            requires_content_evidence: true,
            locator_kind: crate::OutputLocatorKind::Filename,
            ..Default::default()
        },
    };
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        fuzzy_locator_suggestions: vec![
            "/tmp/a/Cargo.toml".to_string(),
            "/tmp/b/Cargo.toml".to_string(),
        ],
        ..Default::default()
    };
    assert_eq!(preferred_route_clarify_question(Some(&ctx)), None);
    let context = route_structured_clarify_context(Some(&ctx)).expect("structured context");
    assert!(context.contains("clarify_case: fuzzy_locator_candidates"));
    assert!(context.contains("candidate_1: /tmp/a/Cargo.toml"));
    assert!(context.contains("candidate_2: /tmp/b/Cargo.toml"));
}

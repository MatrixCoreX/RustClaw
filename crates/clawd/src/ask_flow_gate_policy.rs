use super::*;

pub(super) fn output_contract_requires_planner_execution(
    contract: &crate::IntentOutputContract,
) -> bool {
    contract.requires_content_evidence
        || contract.delivery_required
        || !matches!(contract.locator_kind, crate::OutputLocatorKind::None)
        || !matches!(contract.delivery_intent, crate::OutputDeliveryIntent::None)
        || !matches!(contract.semantic_kind, crate::OutputSemanticKind::None)
}

pub(super) fn bound_direct_answer_candidate_satisfies_output_contract(
    contract: &crate::IntentOutputContract,
) -> bool {
    !contract.delivery_required
        && matches!(contract.locator_kind, crate::OutputLocatorKind::None)
        && matches!(contract.delivery_intent, crate::OutputDeliveryIntent::None)
        && matches!(contract.semantic_kind, crate::OutputSemanticKind::None)
}

pub(super) fn transform_skill_available_for_plan(state: &AppState) -> bool {
    let enabled_skills = state.get_skills_list();
    enabled_skills.is_empty() || enabled_skills.contains("transform")
}

pub(super) fn package_manager_skill_available_for_plan(state: &AppState) -> bool {
    let enabled_skills = state.get_skills_list();
    enabled_skills.is_empty() || enabled_skills.contains("package_manager")
}

pub(super) fn package_manager_skill_supports_detection(state: &AppState) -> bool {
    if !package_manager_skill_available_for_plan(state) {
        return false;
    }
    let Some(manifest) = state.skill_manifest("package_manager") else {
        return true;
    };
    manifest
        .semantic_tags
        .iter()
        .any(|tag| tag == "package_manager_detection")
        || manifest
            .planner_capabilities
            .iter()
            .any(|capability| capability.name == "package.detect_manager")
}

pub(super) fn output_contract_requests_package_manager_detection(
    contract: &crate::IntentOutputContract,
) -> bool {
    matches!(
        contract.semantic_kind,
        crate::OutputSemanticKind::PackageManagerDetection
    )
}

pub(super) fn route_has_package_manager_install_preview_candidate(
    route: &crate::RouteResult,
) -> bool {
    normalizer_answer_candidate_from_resolved_prompt(&route.resolved_intent).is_some_and(
        |candidate| {
            crate::package_commands::package_install_packages_from_commandish_text(&candidate)
                .is_some()
        },
    )
}

pub(super) fn direct_answer_gate_can_skip_for_self_contained_payload(
    current_user_request: &str,
    route: Option<&crate::RouteResult>,
) -> bool {
    let Some(route) = route else {
        return false;
    };
    if normalizer_answer_candidate_from_resolved_prompt(&route.resolved_intent).is_none() {
        return false;
    }
    if route.needs_clarify
        || route.is_execute_gate()
        || route
            .route_confidence
            .is_none_or(|confidence| confidence < 0.80)
        || route.wants_file_delivery
        || !matches!(route.schedule_kind, crate::ScheduleKind::None)
        || output_contract_requires_planner_execution(&route.output_contract)
        || !route.output_contract.locator_hint.trim().is_empty()
        || !matches!(
            route.output_contract.self_extension.mode,
            crate::SelfExtensionMode::None
        )
        || !matches!(
            route.output_contract.self_extension.trigger,
            crate::SelfExtensionTrigger::None
        )
        || route.output_contract.self_extension.execute_now
    {
        return false;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(current_user_request);
    if crate::intent::surface_signals::inline_json_transform_request(current_user_request) {
        return false;
    }
    surface.inline_json_shape.is_some()
        && !surface.has_explicit_path_or_url()
        && !surface.has_filename_candidates()
        && !surface.has_delivery_token_reference()
}

pub(super) fn normalized_workspace_identity_token(text: &str) -> String {
    text.chars()
        .filter_map(|ch| {
            if ch.is_ascii_alphanumeric() {
                Some(ch.to_ascii_lowercase())
            } else if ('\u{4e00}'..='\u{9fff}').contains(&ch) {
                Some(ch)
            } else {
                None
            }
        })
        .collect()
}

pub(super) fn workspace_identity_tokens(state: &AppState) -> Vec<String> {
    let Some(root_name) = state
        .skill_rt
        .workspace_root
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
    else {
        return Vec::new();
    };
    let token = normalized_workspace_identity_token(root_name);
    if token.chars().count() < 4 {
        return Vec::new();
    }
    vec![token]
}

pub(super) fn current_request_mentions_workspace_identity(
    state: &AppState,
    current_user_request: &str,
) -> bool {
    let request = normalized_workspace_identity_token(current_user_request);
    if request.is_empty() {
        return false;
    }
    workspace_identity_tokens(state)
        .into_iter()
        .any(|token| request.contains(&token))
}

pub(super) fn direct_answer_gate_can_skip_for_pure_chat_draft(
    state: &AppState,
    current_user_request: &str,
    route: Option<&crate::RouteResult>,
) -> bool {
    let Some(route) = route else {
        return false;
    };
    if normalizer_answer_candidate_from_resolved_prompt(&route.resolved_intent).is_none()
        || route.needs_clarify
        || route.is_execute_gate()
        || route
            .route_confidence
            .is_none_or(|confidence| confidence < 0.80)
        || route.wants_file_delivery
        || !matches!(route.schedule_kind, crate::ScheduleKind::None)
        || matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar
        )
        || route.output_contract.requires_content_evidence
        || !direct_answer_gate_contract_is_pure_chat(&route.output_contract)
    {
        return false;
    }
    if current_request_mentions_workspace_identity(state, current_user_request) {
        return false;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(current_user_request);
    surface.inline_json_shape.is_none()
        && !crate::intent::surface_signals::inline_json_transform_request(current_user_request)
        && !surface.has_any_locator_reference()
        && !surface.has_filename_candidates()
        && !surface.has_delivery_token_reference()
        && !surface.has_structured_target_refinement()
        && !surface.has_deictic_reference()
        && surface.locator_target_pair.is_none()
}

pub(super) fn direct_answer_gate_can_skip_for_boundary_clean_chat(
    state: &AppState,
    current_user_request: &str,
    route: Option<&crate::RouteResult>,
) -> bool {
    let Some(route) = route else {
        return false;
    };
    if route.needs_clarify
        || route.is_execute_gate()
        || route
            .route_confidence
            .is_none_or(|confidence| confidence < 0.80)
        || route.wants_file_delivery
        || route.should_refresh_long_term_memory
        || !matches!(route.schedule_kind, crate::ScheduleKind::None)
        || matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar
        )
        || !direct_answer_gate_contract_is_pure_chat(&route.output_contract)
    {
        return false;
    }
    if current_request_mentions_workspace_identity(state, current_user_request) {
        return false;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(current_user_request);
    surface.inline_json_shape.is_none()
        && !crate::intent::surface_signals::inline_json_transform_request(current_user_request)
        && !surface.has_any_locator_reference()
        && !surface.has_filename_candidates()
        && !surface.has_delivery_token_reference()
        && !surface.has_structured_target_refinement()
        && !surface.has_deictic_reference()
        && surface.locator_target_pair.is_none()
}

pub(super) fn direct_answer_gate_can_skip_for_standalone_freeform_repair(
    route: Option<&crate::RouteResult>,
) -> bool {
    let Some(route) = route else {
        return false;
    };
    route
        .route_reason
        .contains("standalone_freeform_clarify_downgraded_to_direct_answer")
        && !route.needs_clarify
        && !route.is_execute_gate()
        && !route.wants_file_delivery
        && matches!(route.schedule_kind, crate::ScheduleKind::None)
        && !route.output_contract.requires_content_evidence
        && !route.output_contract.delivery_required
        && matches!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::None
        )
        && matches!(
            route.output_contract.delivery_intent,
            crate::OutputDeliveryIntent::None
        )
        && matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::None
        )
        && direct_answer_gate_contract_is_pure_chat(&route.output_contract)
}

pub(super) fn direct_answer_gate_can_skip_for_active_task_text_mutation(
    current_user_request: &str,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> bool {
    let Some(ctx) = agent_run_context else {
        return false;
    };
    let Some(route) = ctx.route_result.as_ref() else {
        return false;
    };
    let Some(analysis) = ctx.turn_analysis.as_ref() else {
        return false;
    };
    if route.needs_clarify
        || route.is_execute_gate()
        || route.wants_file_delivery
        || !matches!(route.schedule_kind, crate::ScheduleKind::None)
        || output_contract_requires_planner_execution(&route.output_contract)
        || !route.output_contract.locator_hint.trim().is_empty()
        || !matches!(
            route.output_contract.self_extension.mode,
            crate::SelfExtensionMode::None
        )
        || !matches!(
            route.output_contract.self_extension.trigger,
            crate::SelfExtensionTrigger::None
        )
        || route.output_contract.self_extension.execute_now
        || analysis.attachment_processing_required
    {
        return false;
    }
    if !matches!(
        analysis.turn_type,
        Some(
            crate::intent_router::TurnType::TaskAppend
                | crate::intent_router::TurnType::TaskCorrect
                | crate::intent_router::TurnType::TaskReplace
                | crate::intent_router::TurnType::TaskScopeUpdate
        )
    ) || !matches!(
        analysis.target_task_policy,
        Some(
            crate::intent_router::TargetTaskPolicy::ReuseActive
                | crate::intent_router::TargetTaskPolicy::ReplaceActive
        )
    ) {
        return false;
    }

    let surface = crate::intent::surface_signals::analyze_prompt_surface(current_user_request);
    !surface.has_explicit_path_or_url()
        && surface.locator_target_pair.is_none()
        && surface.field_selector_count == 0
        && surface.dotted_field_selector.is_none()
        && !surface.has_delivery_token_reference()
        && surface
            .filename_candidates_excluding_field_selectors()
            .is_empty()
}

pub(super) fn direct_answer_gate_can_skip_for_active_observed_output_chat_repair(
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> bool {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
        return false;
    };
    route
        .route_reason
        .contains("active_observed_output_chat_repair")
        && !route.needs_clarify
        && !route.is_execute_gate()
        && !route.wants_file_delivery
        && matches!(route.schedule_kind, crate::ScheduleKind::None)
        && !output_contract_requires_planner_execution(&route.output_contract)
        && route.output_contract.locator_hint.trim().is_empty()
}

pub(super) fn direct_answer_gate_can_skip_for_recent_execution_judgment_context(
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> bool {
    agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .is_some_and(|route| {
            route_reason_has_exact_marker(route, "clarify_recent_execution_judgment_to_chat")
        })
}

pub(super) fn contract_test_hint_requests_planner_execution(current_user_request: &str) -> bool {
    if crate::intent_router::contract_test_hint_semantic_kind(current_user_request).is_some() {
        return true;
    }
    if crate::intent_router::contract_test_hint_value(current_user_request, "none_passthrough")
        .is_some_and(|value| value.eq_ignore_ascii_case("true"))
    {
        return false;
    }
    let allowed_actions = crate::intent_router::contract_test_hint_value(
        current_user_request,
        "allowed_actions_json",
    )
    .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
    .and_then(|value| {
        value.as_array().map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .any(|item| !item.trim().is_empty())
        })
    })
    .unwrap_or(false);
    let required_evidence = crate::intent_router::contract_test_hint_value(
        current_user_request,
        "required_evidence_json",
    )
    .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
    .and_then(|value| {
        value.as_array().map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .any(|item| !item.trim().is_empty())
        })
    })
    .unwrap_or(false);
    allowed_actions || required_evidence
}

pub(super) fn contract_test_hint_should_enter_planner_loop(
    current_user_request: &str,
    ctx: Option<&crate::agent_engine::AgentRunContext>,
) -> bool {
    if !contract_test_hint_requests_planner_execution(current_user_request) {
        return false;
    }
    ctx.and_then(|ctx| ctx.route_result.as_ref())
        .is_some_and(|route| {
            !route.needs_clarify
                && (route.is_execute_gate()
                    || route.output_contract.requires_content_evidence
                    || route.output_contract.delivery_required
                    || route.wants_file_delivery)
        })
}

pub(super) fn contract_test_hint_forced_planner_preflight(
    ctx: &mut crate::agent_engine::AgentRunContext,
    current_user_request: &str,
    reason_tag: &str,
) -> Option<DirectAnswerPreflight> {
    if !contract_test_hint_should_enter_planner_loop(current_user_request, Some(ctx)) {
        return None;
    }
    if let Some(route) = ctx.route_result.as_mut() {
        let finalize_style = planner_finalize_style_for_output_contract(&route.output_contract);
        route.set_planner_execute_finalize(finalize_style);
        route.needs_clarify = false;
        route.clarify_question.clear();
        append_route_reason(route, reason_tag);
    }
    Some(DirectAnswerPreflight::PlannerExecute(ctx.clone()))
}

pub(super) fn direct_answer_gate_promotion_depends_only_on_background_context(
    state: &AppState,
    current_user_request: &str,
    route: &crate::RouteResult,
    promoted_contract: &crate::IntentOutputContract,
    reference_resolution: &DirectAnswerGateReferenceResolutionOut,
    has_structural_session_alias_target: bool,
) -> bool {
    if has_structural_session_alias_target {
        return false;
    }
    if route.needs_clarify
        || route.is_execute_gate()
        || route.wants_file_delivery
        || !matches!(route.schedule_kind, crate::ScheduleKind::None)
        || output_contract_requires_planner_execution(&route.output_contract)
        || !route.output_contract.locator_hint.trim().is_empty()
        || !output_contract_requires_planner_execution(promoted_contract)
        || promoted_contract.delivery_required
        || !matches!(
            promoted_contract.delivery_intent,
            crate::OutputDeliveryIntent::None
        )
        || !matches!(
            promoted_contract.locator_kind,
            crate::OutputLocatorKind::CurrentWorkspace
                | crate::OutputLocatorKind::Path
                | crate::OutputLocatorKind::Filename
                | crate::OutputLocatorKind::Url
        )
        || !matches!(
            promoted_contract.semantic_kind,
            crate::OutputSemanticKind::None
        )
    {
        return false;
    }
    if normalizer_answer_candidate_from_resolved_prompt(&route.resolved_intent)
        .as_deref()
        .is_some_and(text_mentions_artifact_locator)
    {
        return false;
    }
    if current_request_mentions_workspace_identity(state, current_user_request) {
        return false;
    }
    if (direct_answer_gate_contract_allows_locatorless_execution(
        state,
        current_user_request,
        promoted_contract,
    ) || (package_manager_skill_available_for_plan(state)
        && route_has_package_manager_install_preview_candidate(route)))
        && !direct_answer_gate_reference_requires_clarify(reference_resolution)
    {
        return false;
    }

    let surface = crate::intent::surface_signals::analyze_prompt_surface(current_user_request);
    !direct_answer_gate_reference_is_present(reference_resolution)
        && !current_request_mentions_resolvable_gate_locator(
            state,
            current_user_request,
            promoted_contract,
        )
        && !surface.has_explicit_path_or_url()
        && surface.locator_target_pair.is_none()
        && surface.field_selector_count == 0
        && surface.dotted_field_selector.is_none()
        && !surface.has_delivery_token_reference()
        && surface
            .filename_candidates_excluding_field_selectors()
            .is_empty()
}

pub(super) fn direct_answer_gate_sanitized_freeform_promotion_should_stay_chat(
    state: &AppState,
    current_user_request: &str,
    route: &crate::RouteResult,
    promoted_contract: &crate::IntentOutputContract,
    reference_resolution: &DirectAnswerGateReferenceResolutionOut,
    has_structural_session_alias_target: bool,
) -> bool {
    if has_structural_session_alias_target
        || !route_reason_has_exact_marker(
            route,
            "untrusted_normalizer_freeform_rewrite_removed_from_execution_context",
        )
        || route.needs_clarify
        || route.is_execute_gate()
        || route.wants_file_delivery
        || !matches!(route.schedule_kind, crate::ScheduleKind::None)
        || output_contract_requires_planner_execution(&route.output_contract)
        || !route.output_contract.locator_hint.trim().is_empty()
        || !direct_answer_gate_contract_is_pure_chat(&route.output_contract)
        || !output_contract_requires_planner_execution(promoted_contract)
        || !promoted_contract.requires_content_evidence
        || promoted_contract.delivery_required
        || !matches!(
            promoted_contract.delivery_intent,
            crate::OutputDeliveryIntent::None
        )
        || !matches!(
            promoted_contract.locator_kind,
            crate::OutputLocatorKind::CurrentWorkspace
                | crate::OutputLocatorKind::Path
                | crate::OutputLocatorKind::Filename
                | crate::OutputLocatorKind::Url
        )
        || !matches!(
            promoted_contract.semantic_kind,
            crate::OutputSemanticKind::None | crate::OutputSemanticKind::WorkspaceProjectSummary
        )
        || matches!(
            direct_answer_gate_reference_target(reference_resolution),
            "current_action_result" | "comparison_result"
        )
    {
        return false;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(current_user_request);
    !current_request_mentions_workspace_identity(state, current_user_request)
        && !current_request_has_direct_answer_gate_locator_surface(
            state,
            current_user_request,
            promoted_contract,
        )
        && !surface.has_any_locator_reference()
        && !surface.has_filename_candidates()
        && !surface.has_deictic_reference()
        && surface.locator_target_pair.is_none()
}

pub(super) fn direct_answer_gate_promotion_needs_unbound_deictic_clarify(
    state: &AppState,
    current_user_request: &str,
    auto_locator_path: Option<&str>,
    has_authoritative_deictic_anchor: bool,
    has_structural_session_alias_target: bool,
    contract: &crate::IntentOutputContract,
    reference_resolution: &DirectAnswerGateReferenceResolutionOut,
) -> bool {
    if !output_contract_requires_planner_execution(contract) {
        return false;
    }
    let reference_requires_clarify =
        direct_answer_gate_reference_requires_clarify(reference_resolution);
    if !(matches!(
        contract.locator_kind,
        crate::OutputLocatorKind::Path
            | crate::OutputLocatorKind::Filename
            | crate::OutputLocatorKind::Url
    ) || (contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace
        && reference_requires_clarify))
    {
        return false;
    }
    if current_request_has_direct_answer_gate_locator_surface(state, current_user_request, contract)
    {
        return false;
    }
    if has_authoritative_deictic_anchor || has_structural_session_alias_target {
        return false;
    }
    if auto_locator_path.is_some_and(|path| !path.trim().is_empty()) {
        return false;
    }
    true
}

pub(super) fn direct_answer_gate_untrusted_locator_hint_requires_clarify(
    state: &AppState,
    current_user_request: &str,
    contract: &crate::IntentOutputContract,
    reference_resolution: &DirectAnswerGateReferenceResolutionOut,
    auto_locator_path: Option<&str>,
    has_authoritative_deictic_anchor: bool,
    has_structural_session_alias_target: bool,
) -> bool {
    if !contract.requires_content_evidence
        || contract.locator_hint.trim().is_empty()
        || !matches!(
            contract.locator_kind,
            crate::OutputLocatorKind::Path
                | crate::OutputLocatorKind::Filename
                | crate::OutputLocatorKind::Url
                | crate::OutputLocatorKind::CurrentWorkspace
        )
        || current_request_has_direct_answer_gate_locator_surface(
            state,
            current_user_request,
            contract,
        )
        || has_authoritative_deictic_anchor
        || has_structural_session_alias_target
        || auto_locator_path.is_some_and(|path| !path.trim().is_empty())
    {
        return false;
    }
    matches!(
        direct_answer_gate_reference_target(reference_resolution),
        "" | "none"
            | "current_turn_locator"
            | "unresolved_prior_object"
            | "missing_locator"
            | "ambiguous_locator"
    )
}

pub(super) fn current_request_has_direct_answer_gate_locator_surface(
    state: &AppState,
    current_user_request: &str,
    contract: &crate::IntentOutputContract,
) -> bool {
    let surface = crate::intent::surface_signals::analyze_prompt_surface(current_user_request);
    surface.has_concrete_locator_hint()
        || surface.has_structured_target_refinement()
        || surface.has_delivery_token_reference()
        || (contract.requires_content_evidence
            && matches!(
                contract.locator_kind,
                crate::OutputLocatorKind::Path
                    | crate::OutputLocatorKind::Filename
                    | crate::OutputLocatorKind::CurrentWorkspace
            )
            && current_request_mentions_resolvable_gate_locator(
                state,
                current_user_request,
                contract,
            ))
}

pub(super) fn current_request_mentions_resolvable_gate_locator(
    state: &AppState,
    current_user_request: &str,
    contract: &crate::IntentOutputContract,
) -> bool {
    contract.requires_content_evidence
        && matches!(
            contract.locator_kind,
            crate::OutputLocatorKind::Path
                | crate::OutputLocatorKind::Filename
                | crate::OutputLocatorKind::CurrentWorkspace
        )
        && locator_hint_mentions_current_request(&contract.locator_hint, current_user_request)
        && resolve_gate_locator_from_hint_or_request(state, current_user_request, contract)
            .is_some()
}

pub(super) fn resolve_gate_locator_from_hint_or_request(
    state: &AppState,
    current_user_request: &str,
    contract: &crate::IntentOutputContract,
) -> Option<String> {
    let locator_kind = if contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace {
        crate::OutputLocatorKind::Path
    } else {
        contract.locator_kind
    };
    crate::worker::try_resolve_implicit_locator_path(
        state,
        current_user_request,
        contract.locator_hint.trim(),
        locator_kind,
        None,
    )
    .and_then(|resolution| match resolution {
        crate::worker::LocatorAutoResolution::Direct(path) => Some(path),
        crate::worker::LocatorAutoResolution::Fuzzy(_) => None,
    })
    .or_else(|| {
        crate::worker::try_resolve_workspace_child_locator_from_text(
            &state.skill_rt.workspace_root,
            &state.skill_rt.default_locator_search_dir,
            current_user_request,
        )
    })
}

pub(super) fn locator_hint_mentions_current_request(
    locator_hint: &str,
    current_user_request: &str,
) -> bool {
    let request_lower = current_user_request.to_ascii_lowercase();
    locator_hint
        .split(|ch: char| {
            ch.is_whitespace()
                || matches!(
                    ch,
                    ',' | ';'
                        | ':'
                        | '|'
                        | '/'
                        | '\\'
                        | '('
                        | ')'
                        | '['
                        | ']'
                        | '{'
                        | '}'
                        | '，'
                        | '、'
                        | '；'
                        | '：'
                )
        })
        .map(|token| token.trim_matches(|ch: char| ch == '"' || ch == '\'' || ch == '`'))
        .filter(|token| token.len() >= 3)
        .any(|token| request_lower.contains(&token.to_ascii_lowercase()))
}

pub(super) fn direct_answer_route_introduces_unmentioned_distinctive_context_target(
    current_user_request: &str,
    route: &crate::RouteResult,
    gate: &DirectAnswerGateOut,
) -> bool {
    distinctive_context_tokens(&direct_answer_gate_context_target_text(route, gate))
        .into_iter()
        .any(|token| !distinctive_token_present_in_request(current_user_request, &token))
}

pub(super) fn direct_answer_gate_context_target_text(
    route: &crate::RouteResult,
    gate: &DirectAnswerGateOut,
) -> String {
    let mut text = String::new();
    let (resolved_intent, _) = strip_embedded_answer_candidate_from_intent(&route.resolved_intent);
    text.push_str(&resolved_intent);
    text.push('\n');
    text.push_str(&route.route_reason);
    text.push('\n');
    text.push_str(&gate.resolved_user_intent);
    text.push('\n');
    text.push_str(&gate.reason);
    text
}

pub(super) fn direct_answer_route_introduces_unmentioned_locatorlike_context_target(
    current_user_request: &str,
    route: &crate::RouteResult,
    gate: &DirectAnswerGateOut,
) -> bool {
    let text = direct_answer_gate_context_target_text(route, gate);
    if answer_candidate_introduces_unmentioned_pathlike_target(current_user_request, &text) {
        return true;
    }
    crate::delivery_utils::extract_filename_candidates(&text)
        .into_iter()
        .filter(|candidate| {
            !crate::intent::locator_extractor::candidate_looks_like_dotted_version_number(candidate)
        })
        .any(|candidate| !distinctive_token_present_in_request(current_user_request, &candidate))
}

pub(super) fn distinctive_context_tokens(text: &str) -> Vec<String> {
    text.split(|ch: char| {
        !(ch.is_ascii_alphanumeric()
            || ('\u{4e00}'..='\u{9fff}').contains(&ch)
            || matches!(ch, '_' | '-' | '/' | '.' | ':'))
    })
    .map(|token| token.trim_matches(|ch: char| matches!(ch, '_' | '-' | '/' | '.' | ':')))
    .filter(|token| distinctive_context_token(token))
    .map(ToOwned::to_owned)
    .collect()
}

pub(super) fn distinctive_context_token(token: &str) -> bool {
    let signal_chars = token
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || ('\u{4e00}'..='\u{9fff}').contains(ch))
        .count();
    let has_identifier_separator = token.contains(['_', '/', '.', ':']);
    let has_digit = token.chars().any(|ch| ch.is_ascii_digit());
    (signal_chars >= 4 && has_identifier_separator)
        || (signal_chars >= 8 && has_digit)
        || signal_chars >= 16
}

pub(super) fn distinctive_token_present_in_request(request: &str, token: &str) -> bool {
    let request = request.to_ascii_lowercase();
    let token = token.to_ascii_lowercase();
    if request.contains(&token) {
        return true;
    }
    token
        .split(['_', '-', '/', '.', ':'])
        .filter(|part| part.len() >= 3)
        .any(|part| request.contains(part))
}

pub(super) fn answer_candidate_pathlike_tokens(candidate: &str) -> Vec<String> {
    candidate
        .split_whitespace()
        .map(|token| {
            token.trim_matches(|ch: char| {
                ch.is_ascii_punctuation() && !matches!(ch, '/' | '\\' | '.' | '_' | '-' | '~' | ':')
            })
        })
        .filter(|token| token_looks_like_pathlike_locator(token))
        .filter(|token| distinctive_context_token(token))
        .map(ToOwned::to_owned)
        .collect()
}

pub(super) fn token_looks_like_pathlike_locator(token: &str) -> bool {
    let token = token.trim();
    if token.is_empty() || token.contains(char::is_whitespace) {
        return false;
    }
    if token.contains("://")
        || token.contains('\\')
        || token.starts_with("~/")
        || token.starts_with("./")
        || token.starts_with("../")
        || (token.starts_with('/') && token.len() > 1)
    {
        return true;
    }
    let bytes = token.as_bytes();
    if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\')
    {
        return true;
    }
    if !token.contains('/') {
        return false;
    }
    let parts = token.split('/').collect::<Vec<_>>();
    parts.len() >= 2
        && parts
            .iter()
            .all(|part| token_path_component_looks_structural(part))
}

pub(super) fn token_path_component_looks_structural(part: &str) -> bool {
    let part = part.trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`'));
    !part.is_empty()
        && part.chars().any(|ch| ch.is_ascii_alphanumeric())
        && part
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
}

pub(super) fn answer_candidate_introduces_unmentioned_pathlike_target(
    current_user_request: &str,
    candidate: &str,
) -> bool {
    let request = current_user_request.to_ascii_lowercase();
    answer_candidate_pathlike_tokens(candidate)
        .into_iter()
        .map(|token| token.to_ascii_lowercase())
        .any(|token| {
            if request.contains(&token) {
                return false;
            }
            let basename = token
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or(token.as_str())
                .trim();
            basename.is_empty() || !request.contains(basename)
        })
}

pub(super) fn direct_answer_gate_contract_is_pure_chat(
    contract: &crate::IntentOutputContract,
) -> bool {
    !output_contract_requires_planner_execution(contract)
        && !matches!(
            contract.response_shape,
            crate::OutputResponseShape::FileToken
        )
        && contract.locator_hint.trim().is_empty()
        && !contract.self_extension.execute_now
        && matches!(contract.self_extension.mode, crate::SelfExtensionMode::None)
        && matches!(
            contract.self_extension.trigger,
            crate::SelfExtensionTrigger::None
        )
}

pub(crate) fn route_allows_agent_loop_pure_chat_submode(route: &crate::RouteResult) -> bool {
    route.is_chat_gate()
        && !route.needs_clarify
        && !route.wants_file_delivery
        && !route.should_refresh_long_term_memory
        && route.risk_ceiling != crate::RiskCeiling::High
        && matches!(route.schedule_kind, crate::ScheduleKind::None)
        && direct_answer_gate_contract_is_pure_chat(&route.output_contract)
}

pub(super) fn direct_answer_gate_self_contained_inline_json_chat(
    current_user_request: &str,
) -> bool {
    let surface = crate::intent::surface_signals::analyze_prompt_surface(current_user_request);
    surface.inline_json_shape.is_some()
        && !crate::intent::surface_signals::inline_json_transform_request(current_user_request)
        && !surface.has_explicit_path_or_url()
        && !surface.has_delivery_token_reference()
        && surface.locator_target_pair.is_none()
}

pub(super) fn direct_answer_gate_contextual_inline_structured_payload_execute(
    current_user_request: &str,
    contract: &crate::IntentOutputContract,
) -> bool {
    let surface = crate::intent::surface_signals::analyze_prompt_surface(current_user_request);
    surface.inline_json_shape.is_some()
        && !crate::intent::surface_signals::inline_json_transform_request(current_user_request)
        && !surface.has_explicit_path_or_url()
        && !surface.has_filename_candidates()
        && !surface.has_delivery_token_reference()
        && surface.locator_target_pair.is_none()
        && contract.requires_content_evidence
        && !contract.delivery_required
        && matches!(contract.delivery_intent, crate::OutputDeliveryIntent::None)
        && matches!(contract.locator_kind, crate::OutputLocatorKind::None)
        && contract.locator_hint.trim().is_empty()
}

pub(super) fn direct_answer_gate_embedded_inline_json_payload_surface(
    current_user_request: &str,
) -> bool {
    let surface = crate::intent::surface_signals::analyze_prompt_surface(current_user_request);
    matches!(
        surface.inline_json_shape,
        Some(crate::intent::surface_signals::InlineJsonShape::EmbeddedPayload)
    ) && !surface.has_explicit_path_or_url()
        && !surface.has_filename_candidates()
        && !surface.has_delivery_token_reference()
        && surface.locator_target_pair.is_none()
}

pub(super) fn direct_answer_gate_structured_transform_candidate(answer_candidate: &str) -> bool {
    let normalized = direct_answer_gate_strip_single_markdown_code_fence(answer_candidate);
    let trimmed = normalized.trim();
    if trimmed.is_empty() {
        return false;
    }
    serde_json::from_str::<serde_json::Value>(trimmed)
        .ok()
        .is_some_and(|value| {
            matches!(
                value,
                serde_json::Value::Array(_) | serde_json::Value::Object(_)
            )
        })
        || direct_answer_gate_answer_candidate_is_markdown_table(trimmed)
}

pub(super) fn direct_answer_gate_strip_single_markdown_code_fence(candidate: &str) -> String {
    let trimmed = candidate.trim();
    let lines = trimmed.lines().collect::<Vec<_>>();
    if lines.len() < 3 {
        return trimmed.to_string();
    }
    let first = lines.first().map(|line| line.trim()).unwrap_or_default();
    let last = lines.last().map(|line| line.trim()).unwrap_or_default();
    if first.starts_with("```") && last == "```" {
        lines[1..lines.len() - 1].join("\n").trim().to_string()
    } else {
        trimmed.to_string()
    }
}

pub(super) fn direct_answer_gate_answer_candidate_is_markdown_table(candidate: &str) -> bool {
    let lines = candidate
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    lines.len() >= 2
        && lines
            .first()
            .is_some_and(|line| line.starts_with('|') && line.ends_with('|'))
        && lines
            .get(1)
            .is_some_and(|line| line.chars().all(|ch| matches!(ch, '|' | '-' | ':' | ' ')))
}

pub(super) fn direct_answer_gate_has_recent_observed_result(
    ctx: &crate::agent_engine::AgentRunContext,
) -> bool {
    let Some(context) = ctx
        .cross_turn_recent_execution_context
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "<none>")
    else {
        return false;
    };
    context.contains("latest_result=") || context.contains(" result=")
}

pub(super) fn direct_answer_gate_existing_observed_result_should_stay_chat(
    current_user_request: &str,
    route: &crate::RouteResult,
    gate: &DirectAnswerGateOut,
    ctx: &crate::agent_engine::AgentRunContext,
) -> bool {
    if route.needs_clarify
        || route.is_execute_gate()
        || route.wants_file_delivery
        || !matches!(route.schedule_kind, crate::ScheduleKind::None)
        || route.output_contract.delivery_required
        || !matches!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::None
        )
        || !route.output_contract.locator_hint.trim().is_empty()
        || !matches!(
            route.output_contract.delivery_intent,
            crate::OutputDeliveryIntent::None
        )
        || !matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::None
                | crate::OutputSemanticKind::ContentExcerptSummary
                | crate::OutputSemanticKind::RawCommandOutput
                | crate::OutputSemanticKind::ContentPresenceCheck
        )
        || !direct_answer_gate_has_recent_observed_result(ctx)
    {
        return false;
    }
    let reference_target = gate.reference_resolution.target.trim();
    let normalizer_answer_candidate_present =
        normalizer_answer_candidate_from_resolved_prompt(&route.resolved_intent).is_some();
    let reference_binds_observed_result = matches!(
        reference_target,
        "current_action_result" | "comparison_result"
    ) || (normalizer_answer_candidate_present
        && matches!(
            reference_target,
            "unresolved_prior_object" | "missing_locator" | "ambiguous_locator"
        ));
    if !reference_binds_observed_result {
        return false;
    }
    let gate_locator_kind = gate.output_contract.locator_kind.trim();
    let gate_delivery_intent = gate.output_contract.delivery_intent.trim();
    let gate_locator_hint = gate.output_contract.locator_hint.trim();
    let gate_semantic_kind = gate.output_contract.semantic_kind.trim();
    if gate.output_contract.delivery_required
        || !gate_locator_kind.is_empty() && gate_locator_kind != "none"
        || !gate_delivery_intent.is_empty() && gate_delivery_intent != "none"
        || !gate_locator_hint.is_empty()
        || !matches!(
            gate_semantic_kind,
            "" | "none"
                | "content_excerpt_summary"
                | "raw_command_output"
                | "command_output_summary"
                | "content_presence_check"
        )
    {
        return false;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(current_user_request);
    !surface.has_concrete_locator_hint()
        && !surface.has_explicit_path_or_url()
        && !surface.has_filename_candidates()
        && !surface.has_delivery_token_reference()
        && !surface.has_structured_target_refinement()
        && surface.locator_target_pair.is_none()
        && !crate::intent::surface_signals::inline_json_transform_request(current_user_request)
}

pub(super) fn direct_answer_gate_allows_contextual_chat_reference(
    current_user_request: &str,
    route: &crate::RouteResult,
    gate: &DirectAnswerGateOut,
) -> bool {
    if parse_direct_answer_gate_decision(&gate.decision) != DirectAnswerGateDecision::DirectAnswer
        || route.needs_clarify
        || route.is_execute_gate()
        || route.wants_file_delivery
        || !matches!(route.schedule_kind, crate::ScheduleKind::None)
        || direct_answer_gate_reference_requires_clarify(&gate.reference_resolution)
        || !gate.clarify_question.trim().is_empty()
        || direct_answer_route_introduces_unmentioned_locatorlike_context_target(
            current_user_request,
            route,
            gate,
        )
    {
        return false;
    }
    let gate_contract = output_contract_from_direct_answer_gate(
        gate.output_contract.clone(),
        &route.output_contract,
    );
    if !direct_answer_gate_contract_is_pure_chat(&route.output_contract)
        || !direct_answer_gate_contract_is_pure_chat(&gate_contract)
    {
        return false;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(current_user_request);
    if direct_answer_gate_self_contained_inline_json_chat(current_user_request) {
        return true;
    }
    !surface.has_concrete_locator_hint()
        && !surface.is_structural_locator_only_reply()
        && !surface.has_structured_target_refinement()
        && !surface.has_delivery_token_reference()
        && !surface.has_filename_candidates()
        && surface.locator_target_pair.is_none()
}

pub(super) fn direct_answer_gate_candidate_needs_unbound_context_clarify(
    state: &AppState,
    current_user_request: &str,
    route: &crate::RouteResult,
    gate: &DirectAnswerGateOut,
    auto_locator_path: Option<&str>,
    has_authoritative_deictic_anchor: bool,
    has_structural_session_alias_target: bool,
    normalizer_candidate_matches_bound_context: bool,
) -> bool {
    let candidate = normalizer_answer_candidate_from_resolved_prompt(&route.resolved_intent);
    if route.output_contract.semantic_kind == crate::OutputSemanticKind::FileBasename
        && ask_route_reason_has_marker(route, "active_file_basename_answer_candidate_direct")
        && candidate
            .as_deref()
            .is_some_and(|value| single_component_basename_candidate(value).is_some())
    {
        return false;
    }
    if route.needs_clarify
        || route.is_execute_gate()
        || has_authoritative_deictic_anchor
        || has_structural_session_alias_target
        || auto_locator_path.is_some_and(|path| !path.trim().is_empty())
        || current_request_has_direct_answer_gate_locator_surface(
            state,
            current_user_request,
            &route.output_contract,
        )
    {
        return false;
    }
    let Some(candidate) = candidate else {
        let gate_contract = output_contract_from_direct_answer_gate(
            gate.output_contract.clone(),
            &route.output_contract,
        );
        if direct_answer_gate_self_contained_inline_json_chat(current_user_request)
            && parse_direct_answer_gate_decision(&gate.decision)
                == DirectAnswerGateDecision::DirectAnswer
            && gate.clarify_question.trim().is_empty()
            && !direct_answer_gate_reference_requires_clarify(&gate.reference_resolution)
            && direct_answer_gate_contract_is_pure_chat(&route.output_contract)
            && direct_answer_gate_contract_is_pure_chat(&gate_contract)
        {
            return false;
        }
        if direct_answer_gate_allows_contextual_chat_reference(current_user_request, route, gate) {
            return false;
        }
        let reference_requires_clarify =
            direct_answer_gate_reference_requires_clarify(&gate.reference_resolution);
        if !reference_requires_clarify
            && !current_request_has_context_binding_surface(current_user_request)
        {
            return false;
        }
        return direct_answer_route_introduces_unmentioned_distinctive_context_target(
            current_user_request,
            route,
            gate,
        );
    };
    if normalizer_candidate_matches_bound_context
        || normalizer_answer_candidate_matches_runtime_fact(state, &candidate)
    {
        return false;
    }
    if direct_answer_gate_allows_contextual_chat_reference(current_user_request, route, gate)
        && !answer_candidate_introduces_unmentioned_pathlike_target(
            current_user_request,
            &candidate,
        )
    {
        return false;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(current_user_request);
    if !surface.has_concrete_locator_hint()
        && !surface.has_structured_target_refinement()
        && !surface.has_delivery_token_reference()
        && !surface.has_filename_candidates()
        && !surface.has_deictic_reference()
    {
        return false;
    }
    direct_answer_route_introduces_unmentioned_distinctive_context_target(
        current_user_request,
        route,
        gate,
    ) || answer_candidate_introduces_unmentioned_pathlike_target(current_user_request, &candidate)
}

pub(super) fn direct_answer_gate_contract_allows_locatorless_execution(
    state: &AppState,
    current_user_request: &str,
    contract: &crate::IntentOutputContract,
) -> bool {
    if crate::intent::surface_signals::inline_json_transform_request(current_user_request) {
        return true;
    }
    match contract.semantic_kind {
        crate::OutputSemanticKind::PackageManagerDetection => {
            package_manager_skill_supports_detection(state)
        }
        crate::OutputSemanticKind::None
            if matches!(contract.response_shape, crate::OutputResponseShape::Scalar) =>
        {
            true
        }
        crate::OutputSemanticKind::RawCommandOutput => {
            crate::agent_engine::explicit_command_segment_for_policy(
                &state.policy.command_intent,
                current_user_request.trim(),
            )
            .is_some()
        }
        crate::OutputSemanticKind::ServiceStatus
        | crate::OutputSemanticKind::WorkspaceProjectSummary
        | crate::OutputSemanticKind::GitCommitSubject
        | crate::OutputSemanticKind::GitRepositoryState
        | crate::OutputSemanticKind::ToolDiscovery
        | crate::OutputSemanticKind::DockerPs
        | crate::OutputSemanticKind::DockerImages
        | crate::OutputSemanticKind::DockerLogs
        | crate::OutputSemanticKind::DockerContainerLifecycle
        | crate::OutputSemanticKind::WeatherQuery
        | crate::OutputSemanticKind::MarketQuote
        | crate::OutputSemanticKind::ImageUnderstanding
        | crate::OutputSemanticKind::PublishingPreview => true,
        _ => false,
    }
}

pub(super) fn direct_answer_gate_planner_needs_unbound_locator_clarify(
    state: &AppState,
    current_user_request: &str,
    contract: &crate::IntentOutputContract,
    reference_resolution: &DirectAnswerGateReferenceResolutionOut,
    auto_locator_path: Option<&str>,
    has_authoritative_deictic_anchor: bool,
) -> bool {
    if !contract.requires_content_evidence
        || contract.delivery_required
        || !matches!(contract.locator_kind, crate::OutputLocatorKind::None)
        || !contract.locator_hint.trim().is_empty()
        || !direct_answer_gate_reference_is_present(reference_resolution)
        || (direct_answer_gate_reference_is_present(reference_resolution)
            && !direct_answer_gate_reference_requires_clarify(reference_resolution))
        || current_request_has_direct_answer_gate_locator_surface(
            state,
            current_user_request,
            contract,
        )
        || has_authoritative_deictic_anchor
        || auto_locator_path.is_some_and(|path| !path.trim().is_empty())
    {
        return false;
    }
    !direct_answer_gate_contract_allows_locatorless_execution(state, current_user_request, contract)
}

pub(super) fn direct_answer_gate_delivery_needs_unbound_existing_file_clarify(
    state: &AppState,
    current_user_request: &str,
    contract: &crate::IntentOutputContract,
    auto_locator_path: Option<&str>,
    has_authoritative_deictic_anchor: bool,
    has_structural_session_alias_target: bool,
) -> bool {
    let requires_file_delivery = contract.delivery_required
        || matches!(
            contract.response_shape,
            crate::OutputResponseShape::FileToken
        )
        || matches!(
            contract.delivery_intent,
            crate::OutputDeliveryIntent::FileSingle
        );
    if !requires_file_delivery
        || matches!(
            contract.semantic_kind,
            crate::OutputSemanticKind::GeneratedFileDelivery
        )
        || current_request_has_direct_answer_gate_locator_surface(
            state,
            current_user_request,
            contract,
        )
        || has_authoritative_deictic_anchor
        || has_structural_session_alias_target
        || auto_locator_path.is_some_and(|path| !path.trim().is_empty())
    {
        return false;
    }
    true
}

pub(super) fn direct_answer_gate_reference_target(
    reference_resolution: &DirectAnswerGateReferenceResolutionOut,
) -> &str {
    reference_resolution.target.trim()
}

pub(super) fn direct_answer_gate_reference_is_present(
    reference_resolution: &DirectAnswerGateReferenceResolutionOut,
) -> bool {
    !matches!(
        direct_answer_gate_reference_target(reference_resolution),
        "" | "none"
    )
}

pub(super) fn direct_answer_gate_reference_requires_clarify(
    reference_resolution: &DirectAnswerGateReferenceResolutionOut,
) -> bool {
    matches!(
        direct_answer_gate_reference_target(reference_resolution),
        "unresolved_prior_object" | "missing_locator" | "ambiguous_locator"
    )
}

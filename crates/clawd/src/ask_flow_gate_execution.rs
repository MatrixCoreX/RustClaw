use super::*;

pub(super) fn planner_finalize_style_for_output_contract(
    contract: &crate::IntentOutputContract,
) -> ActFinalizeStyle {
    if let Some(style) =
        crate::post_route_policy::content_evidence_execution_finalize_style(contract, false)
    {
        return style;
    }
    if matches!(
        contract.response_shape,
        crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
    ) {
        ActFinalizeStyle::Plain
    } else {
        ActFinalizeStyle::ChatWrapped
    }
}

pub(super) fn promote_direct_answer_gate_to_planner(
    ctx: &mut crate::agent_engine::AgentRunContext,
    gate: &DirectAnswerGateOut,
    mut contract: crate::IntentOutputContract,
    reason_tag: &str,
) -> DirectAnswerPreflight {
    let Some(route) = ctx.route_result.as_mut() else {
        return DirectAnswerPreflight::DirectAnswer;
    };
    let package_install_preview_candidate = normalizer_answer_candidate_from_resolved_prompt(
        &route.resolved_intent,
    )
    .filter(|candidate| {
        crate::package_commands::package_install_packages_from_commandish_text(candidate).is_some()
    });
    contract.requires_content_evidence = true;
    let finalize_style = planner_finalize_style_for_output_contract(&contract);
    route.output_contract = contract;
    route.set_planner_execute_finalize(finalize_style);
    route.needs_clarify = false;
    route.clarify_question.clear();
    if !gate.resolved_user_intent.trim().is_empty() {
        route.resolved_intent = gate.resolved_user_intent.trim().to_string();
        if let Some(candidate) = package_install_preview_candidate {
            route.resolved_intent.push_str("\nanswer_candidate: ");
            route.resolved_intent.push_str(candidate.trim());
        }
    }
    append_route_reason(route, &format!("{reason_tag}:{}", gate.reason.trim()));
    DirectAnswerPreflight::PlannerExecute(
        ctx.clone(),
        direct_answer_gate_planner_promotion_reason_code(reason_tag),
    )
}

pub(super) fn direct_answer_gate_planner_promotion_reason_code(reason_tag: &str) -> &'static str {
    match reason_tag {
        "direct_answer_gate_inline_transform_execute"
        | "direct_answer_gate_contract_execute"
        | "direct_answer_gate_package_manager_detect_execute"
        | "inline_structured_payload_context_execute" => {
            "direct_answer_gate_contract_boundary_execute"
        }
        "direct_answer_gate_artifact_listing_execute"
        | "direct_answer_gate_recent_file_context_execute"
        | "direct_answer_gate_workspace_child_context_execute" => {
            "direct_answer_gate_evidence_projection_execute"
        }
        _ => "direct_answer_gate_promoted_to_planner",
    }
}

pub(super) fn resolve_direct_answer_gate_contract_locator(
    state: &AppState,
    current_user_request: &str,
    gate: &DirectAnswerGateOut,
    contract: &crate::IntentOutputContract,
    reference_resolution: &DirectAnswerGateReferenceResolutionOut,
) -> Option<String> {
    if !matches!(
        contract.locator_kind,
        crate::OutputLocatorKind::Path
            | crate::OutputLocatorKind::Filename
            | crate::OutputLocatorKind::CurrentWorkspace
    ) {
        return None;
    }
    let hint = contract.locator_hint.trim();
    if contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace && hint.is_empty() {
        return None;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(current_user_request);
    if direct_answer_gate_reference_requires_clarify(reference_resolution)
        && !surface.has_concrete_locator_hint()
        && !surface.has_structured_target_refinement()
        && !surface.has_delivery_token_reference()
    {
        return None;
    }
    let resolved = if hint.is_empty() {
        gate.resolved_user_intent.trim()
    } else {
        hint
    };
    if resolved.is_empty() {
        return None;
    }
    let locator_kind = if contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace {
        crate::OutputLocatorKind::Path
    } else {
        contract.locator_kind
    };
    let direct_resolution = crate::worker::try_resolve_implicit_locator_path(
        state,
        current_user_request,
        resolved,
        locator_kind,
        None,
    )
    .and_then(|resolution| match resolution {
        crate::worker::LocatorAutoResolution::Direct(path) => Some(path),
        crate::worker::LocatorAutoResolution::Fuzzy(_) => None,
    });
    direct_resolution.or_else(|| {
        crate::worker::try_resolve_workspace_child_locator_from_text(
            &state.skill_rt.workspace_root,
            &state.skill_rt.default_locator_search_dir,
            current_user_request,
        )
    })
}

pub(super) fn bind_direct_answer_gate_contract_locator(
    state: &AppState,
    current_user_request: &str,
    gate: &DirectAnswerGateOut,
    contract: &mut crate::IntentOutputContract,
) -> Option<String> {
    let path = resolve_direct_answer_gate_contract_locator(
        state,
        current_user_request,
        gate,
        contract,
        &gate.reference_resolution,
    )?;
    contract.locator_kind = crate::OutputLocatorKind::Path;
    contract.locator_hint = path.clone();
    Some(path)
}

pub(super) trait OutputContractFallbackShape {
    fn with_fallback_shape(self, fallback: &crate::IntentOutputContract) -> Self;
}

impl OutputContractFallbackShape for crate::IntentOutputContract {
    fn with_fallback_shape(mut self, fallback: &crate::IntentOutputContract) -> Self {
        if matches!(self.response_shape, crate::OutputResponseShape::Free)
            && !matches!(fallback.response_shape, crate::OutputResponseShape::Free)
        {
            self.response_shape = fallback.response_shape;
            self.exact_sentence_count = fallback.exact_sentence_count;
        }
        if self.locator_hint.is_empty()
            && matches!(
                self.locator_kind,
                crate::OutputLocatorKind::Path
                    | crate::OutputLocatorKind::Filename
                    | crate::OutputLocatorKind::Url
            )
        {
            self.locator_hint = fallback.locator_hint.clone();
        }
        self
    }
}

pub(super) fn append_route_reason(route: &mut crate::RouteResult, addition: &str) {
    let addition = addition.trim();
    if addition.is_empty() || route.route_reason.contains(addition) {
        return;
    }
    if route.route_reason.trim().is_empty() {
        route.route_reason = addition.to_string();
    } else {
        route.route_reason.push_str("; ");
        route.route_reason.push_str(addition);
    }
}

pub(super) fn turn_analysis_has_alias_only_state_patch(
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    turn_analysis
        .and_then(|analysis| analysis.state_patch.as_ref())
        .is_some_and(crate::conversation_state::state_patch_is_alias_bindings_only)
}

pub(super) fn turn_analysis_has_alias_bindings_state_patch(
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    let Some(state_patch) = turn_analysis.and_then(|analysis| analysis.state_patch.as_ref()) else {
        return false;
    };
    !crate::conversation_state::session_alias_bindings_from_state_patch(Some(state_patch))
        .is_empty()
}

pub(super) fn route_is_memory_update_ack_contract(
    route: &crate::RouteResult,
    has_alias_only_state_patch: bool,
) -> bool {
    (route.should_refresh_long_term_memory || has_alias_only_state_patch)
        && route_allows_memory_ack_shape(route)
}

pub(super) fn route_allows_memory_ack_shape(route: &crate::RouteResult) -> bool {
    !route.needs_clarify
        && !route.wants_file_delivery
        && route.is_chat_gate()
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
}

pub(super) fn route_has_executionless_direct_downgrade(route: &crate::RouteResult) -> bool {
    route
        .route_reason
        .contains("executionless_route_downgraded_to_direct_answer")
}

pub(super) fn current_request_has_structural_execution_target(current_user_request: &str) -> bool {
    if crate::intent::surface_signals::inline_json_transform_request(current_user_request) {
        return true;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(current_user_request);
    surface.has_explicit_path_or_url()
        || surface.locator_target_pair.is_some()
        || surface.field_selector_count > 0
        || surface.dotted_field_selector.is_some()
        || surface.has_delivery_token_reference()
        || surface.has_filename_candidates()
}

pub(super) fn current_request_has_context_binding_surface(current_user_request: &str) -> bool {
    let surface = crate::intent::surface_signals::analyze_prompt_surface(current_user_request);
    surface.has_concrete_locator_hint()
        || surface.has_structured_target_refinement()
        || surface.has_delivery_token_reference()
        || surface.has_filename_candidates()
        || surface.locator_target_pair.is_some()
        || surface.has_deictic_reference()
}

pub(super) fn current_request_has_workspace_child_locator_surface(
    current_user_request: &str,
) -> bool {
    let surface = crate::intent::surface_signals::analyze_prompt_surface(current_user_request);
    surface.has_concrete_locator_hint()
        || surface.has_structured_target_refinement()
        || surface.has_delivery_token_reference()
        || surface.has_filename_candidates()
        || surface.locator_target_pair.is_some()
        || surface.has_explicit_path_or_url()
}

pub(super) fn current_request_resolves_workspace_child_locator(
    state: &AppState,
    current_user_request: &str,
) -> Option<String> {
    let surface = crate::intent::surface_signals::analyze_prompt_surface(current_user_request);
    if surface.has_deictic_reference() && !surface.has_explicit_path_or_url() {
        return None;
    }
    crate::worker::try_resolve_workspace_child_locator_from_text(
        &state.skill_rt.workspace_root,
        &state.skill_rt.default_locator_search_dir,
        current_user_request,
    )
}

pub(super) fn explicit_workspace_child_path_token(
    state: &AppState,
    current_user_request: &str,
) -> Option<String> {
    let workspace_root = state
        .skill_rt
        .workspace_root
        .canonicalize()
        .unwrap_or_else(|_| state.skill_rt.workspace_root.clone());
    let default_search_dir = state
        .skill_rt
        .default_locator_search_dir
        .canonicalize()
        .unwrap_or_else(|_| state.skill_rt.default_locator_search_dir.clone());
    for locator in
        crate::intent::locator_extractor::extract_explicit_locator_candidates_for_fallback(
            current_user_request,
        )
    {
        if !matches!(locator.locator_kind, crate::OutputLocatorKind::Path) {
            continue;
        }
        let raw_path = Path::new(locator.locator_hint.trim());
        let candidate = if raw_path.is_absolute() {
            raw_path.to_path_buf()
        } else {
            default_search_dir.join(raw_path)
        };
        let Ok(candidate) = candidate.canonicalize() else {
            continue;
        };
        if paths_refer_to_same_existing_location(&candidate, &workspace_root) {
            continue;
        }
        if candidate.starts_with(&workspace_root) {
            return Some(candidate.to_string_lossy().to_string());
        }
    }
    None
}

pub(super) fn current_request_resolves_workspace_child_locator_surface(
    state: &AppState,
    current_user_request: &str,
) -> Option<String> {
    let surface = crate::intent::surface_signals::analyze_prompt_surface(current_user_request);
    if surface.has_explicit_path_or_url() {
        if let Some(resolved) = crate::worker::try_resolve_implicit_locator_path(
            state,
            current_user_request,
            "",
            crate::OutputLocatorKind::Path,
            None,
        )
        .and_then(|resolution| match resolution {
            crate::worker::LocatorAutoResolution::Direct(path) => Some(path),
            crate::worker::LocatorAutoResolution::Fuzzy(_) => None,
        }) {
            let resolved_path = Path::new(&resolved);
            if !paths_refer_to_same_existing_location(resolved_path, &state.skill_rt.workspace_root)
            {
                return Some(resolved);
            }
        }
        if let Some(resolved) = explicit_workspace_child_path_token(state, current_user_request) {
            return Some(resolved);
        }
    }
    let resolved = current_request_resolves_workspace_child_locator(state, current_user_request)?;
    if current_request_has_workspace_child_locator_surface(current_user_request) {
        return Some(resolved);
    }
    Path::new(&resolved).is_dir().then_some(resolved)
}

pub(super) fn current_request_resolves_structural_workspace_child_locator_surface(
    state: &AppState,
    current_user_request: &str,
) -> Option<String> {
    current_request_has_workspace_child_locator_surface(current_user_request)
        .then(|| {
            current_request_resolves_workspace_child_locator_surface(state, current_user_request)
        })
        .flatten()
}

pub(super) fn direct_answer_gate_chat_promotion_lacks_structured_target(
    state: &AppState,
    current_user_request: &str,
    route: &crate::RouteResult,
    contract: &crate::IntentOutputContract,
    reference_resolution: &DirectAnswerGateReferenceResolutionOut,
    has_structural_session_alias_target: bool,
) -> bool {
    if !route.is_chat_gate()
        || route.needs_clarify
        || has_structural_session_alias_target
        || (package_manager_skill_available_for_plan(state)
            && route_has_package_manager_install_preview_candidate(route))
        || direct_answer_gate_reference_requires_clarify(reference_resolution)
        || direct_answer_gate_contract_allows_locatorless_execution(
            state,
            current_user_request,
            contract,
        )
        || current_request_mentions_resolvable_gate_locator(state, current_user_request, contract)
        || matches!(contract.locator_kind, crate::OutputLocatorKind::None)
        || current_request_has_structural_execution_target(current_user_request)
        || crate::intent::surface_signals::analyze_prompt_surface(current_user_request)
            .has_deictic_reference()
        || current_request_resolves_structural_workspace_child_locator_surface(
            state,
            current_user_request,
        )
        .is_some()
        || matches!(
            contract.locator_kind,
            crate::OutputLocatorKind::CurrentWorkspace
        )
    {
        return false;
    }
    true
}

pub(super) fn direct_answer_gate_preference_or_memory_promotion_should_stay_chat(
    state: &AppState,
    current_user_request: &str,
    route: &crate::RouteResult,
    contract: &crate::IntentOutputContract,
    reference_resolution: &DirectAnswerGateReferenceResolutionOut,
    turn_type: Option<crate::intent_router::TurnType>,
    has_structural_session_alias_target: bool,
) -> bool {
    if turn_type != Some(crate::intent_router::TurnType::PreferenceOrMemory)
        || !route.is_chat_gate()
        || route.needs_clarify
        || route.is_execute_gate()
        || route.wants_file_delivery
        || has_structural_session_alias_target
        || !matches!(route.schedule_kind, crate::ScheduleKind::None)
        || !output_contract_requires_planner_execution(contract)
        || direct_answer_gate_reference_requires_clarify(reference_resolution)
        || current_request_has_structural_execution_target(current_user_request)
        || current_request_has_direct_answer_gate_locator_surface(
            state,
            current_user_request,
            contract,
        )
        || current_request_mentions_resolvable_gate_locator(state, current_user_request, contract)
        || current_request_resolves_structural_workspace_child_locator_surface(
            state,
            current_user_request,
        )
        .is_some()
    {
        return false;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(current_user_request);
    !surface.has_explicit_path_or_url()
        && !surface.has_concrete_locator_hint()
        && !surface.has_filename_candidates()
        && !surface.has_delivery_token_reference()
        && surface.locator_target_pair.is_none()
        && surface.field_selector_count == 0
        && surface.dotted_field_selector.is_none()
}

pub(super) fn direct_answer_gate_promotes_workspace_child_context(
    state: &AppState,
    current_user_request: &str,
    route: &crate::RouteResult,
    contract: &mut crate::IntentOutputContract,
) -> bool {
    if route.needs_clarify
        || route.is_execute_gate()
        || route.wants_file_delivery
        || route.should_refresh_long_term_memory
        || !matches!(route.schedule_kind, crate::ScheduleKind::None)
        || output_contract_requires_planner_execution(contract)
        || !matches!(
            contract.locator_kind,
            crate::OutputLocatorKind::None | crate::OutputLocatorKind::CurrentWorkspace
        )
        || !contract.locator_hint.trim().is_empty()
    {
        return false;
    }
    let Some(path) = current_request_resolves_structural_workspace_child_locator_surface(
        state,
        current_user_request,
    ) else {
        return false;
    };
    contract.requires_content_evidence = true;
    contract.locator_kind = crate::OutputLocatorKind::Path;
    contract.locator_hint = path;
    true
}

pub(super) fn structural_session_alias_locator_for_target(
    target: &str,
) -> Option<crate::intent::locator_extractor::ExtractedLocator> {
    let target = target.trim();
    if target.is_empty() {
        return None;
    }
    crate::intent::locator_extractor::extract_explicit_locator_for_fallback(target)
}

pub(super) fn current_request_structural_session_alias_locator(
    ctx: &crate::agent_engine::AgentRunContext,
    current_user_request: &str,
) -> Option<crate::intent::locator_extractor::ExtractedLocator> {
    let binding = crate::conversation_state::single_alias_binding_mentioned_in_prompt(
        &ctx.session_alias_bindings,
        current_user_request,
    )?;
    structural_session_alias_locator_for_target(&binding.target)
}

pub(super) fn bind_session_alias_locator_to_contract(
    locator: Option<&crate::intent::locator_extractor::ExtractedLocator>,
    contract: &mut crate::IntentOutputContract,
) {
    let Some(locator) = locator else {
        return;
    };
    contract.requires_content_evidence = true;
    contract.locator_kind = locator.locator_kind;
    contract.locator_hint = locator.locator_hint.clone();
}

pub(super) fn normalized_schema_tokens(raw: &str) -> Vec<String> {
    raw.trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect::<String>()
        .split('_')
        .filter(|token| !token.is_empty())
        .map(ToString::to_string)
        .collect()
}

pub(super) fn resolved_intent_declares_structured_scalar_extraction(resolved_intent: &str) -> bool {
    let (stripped, _) = strip_embedded_answer_candidate_from_intent(resolved_intent);
    let trimmed = stripped.trim();
    if trimmed.is_empty() {
        return false;
    }
    trimmed.lines().any(|line| {
        let line = line.trim();
        if line.is_empty() || line.chars().any(char::is_whitespace) {
            return false;
        }
        let tokens = normalized_schema_tokens(line);
        tokens.iter().any(|token| {
            matches!(
                token.as_str(),
                "scalar" | "title" | "heading" | "subject" | "value"
            )
        }) || tokens.windows(2).any(|pair| {
            matches!(
                (&pair[0][..], &pair[1][..]),
                ("extract", "title") | ("extract", "scalar") | ("first", "heading")
            )
        })
    })
}

pub(super) fn preserve_structured_scalar_extraction_contract(
    contract: &mut crate::IntentOutputContract,
    structured_scalar_extraction: bool,
) {
    if !structured_scalar_extraction || contract.delivery_required {
        return;
    }
    contract.requires_content_evidence = true;
    contract.response_shape = crate::OutputResponseShape::Scalar;
    if matches!(
        contract.semantic_kind,
        crate::OutputSemanticKind::ContentExcerptSummary
    ) {
        contract.semantic_kind = crate::OutputSemanticKind::None;
    }
}

pub(super) fn apply_direct_answer_gate_outcome(
    state: &AppState,
    ctx: &mut crate::agent_engine::AgentRunContext,
    current_user_request: &str,
    gate: DirectAnswerGateOut,
) -> DirectAnswerPreflight {
    let mut gate = gate;
    if let Some(preflight) = contract_test_hint_forced_planner_preflight(
        ctx,
        current_user_request,
        "direct_answer_gate_contract_hint_forced_planner",
    ) {
        return preflight;
    }
    let decision = parse_direct_answer_gate_decision(&gate.decision);
    if gate.confidence < 0.60 {
        return DirectAnswerPreflight::DirectAnswer;
    }
    merge_direct_answer_gate_state_patch(ctx, gate.state_patch.as_ref());
    let recent_request_file_target_count =
        collect_recent_execution_request_file_targets(state, Some(ctx)).len();
    let has_alias_only_state_patch =
        turn_analysis_has_alias_only_state_patch(ctx.turn_analysis.as_ref());
    let has_alias_bindings_state_patch =
        turn_analysis_has_alias_bindings_state_patch(ctx.turn_analysis.as_ref());
    let has_alias_state_patch = has_alias_only_state_patch || has_alias_bindings_state_patch;
    let structural_session_alias_locator =
        current_request_structural_session_alias_locator(ctx, current_user_request);
    let has_structural_session_alias_target = structural_session_alias_locator.is_some();
    let normalizer_candidate_matches_bound_context = ctx
        .route_result
        .as_ref()
        .and_then(|route| normalizer_answer_candidate_from_resolved_prompt(&route.resolved_intent))
        .is_some_and(|candidate| {
            normalizer_answer_candidate_matches_bound_runtime_context(state, &candidate, Some(ctx))
        });
    let existing_observed_result_should_stay_chat =
        ctx.route_result.as_ref().is_some_and(|route| {
            direct_answer_gate_existing_observed_result_should_stay_chat(
                current_user_request,
                route,
                &gate,
                ctx,
            )
        });
    let preserve_active_task_text_mutation =
        direct_answer_gate_can_skip_for_active_task_text_mutation(current_user_request, Some(ctx));
    let turn_type = ctx
        .turn_analysis
        .as_ref()
        .and_then(|analysis| analysis.turn_type);
    let Some(route) = ctx.route_result.as_mut() else {
        return DirectAnswerPreflight::DirectAnswer;
    };
    if crate::agent_engine::agent_loop_authority_selected_migration_class(state, route).is_some() {
        append_route_reason(route, "direct_answer_gate_demoted_for_agent_loop_authority");
        return DirectAnswerPreflight::PlannerExecute(
            ctx.clone(),
            "direct_answer_gate_agent_loop_activation",
        );
    }
    let structured_scalar_extraction =
        resolved_intent_declares_structured_scalar_extraction(&route.resolved_intent);
    let auto_locator_path = ctx.auto_locator_path.as_deref();
    let has_authoritative_deictic_anchor = ctx.has_authoritative_deictic_anchor;
    let force_inline_transform_execution = transform_skill_available_for_plan(state)
        && (crate::intent::surface_signals::inline_json_transform_request(current_user_request)
            || (direct_answer_gate_embedded_inline_json_payload_surface(current_user_request)
                && (decision == DirectAnswerGateDecision::PlannerExecute
                    || normalizer_answer_candidate_from_resolved_prompt(&route.resolved_intent)
                        .as_deref()
                        .is_some_and(direct_answer_gate_structured_transform_candidate))));
    let force_package_manager_detect_execution = package_manager_skill_supports_detection(state)
        && output_contract_requests_package_manager_detection(&route.output_contract);
    let force_package_manager_install_preview_execution =
        package_manager_skill_available_for_plan(state)
            && route_has_package_manager_install_preview_candidate(route);
    if route_is_memory_update_ack_contract(route, has_alias_state_patch) {
        append_route_reason(route, "direct_answer_gate_memory_update_ignored");
        return DirectAnswerPreflight::DirectAnswer;
    }
    if preserve_active_task_text_mutation {
        append_route_reason(
            route,
            "direct_answer_gate_active_task_text_mutation_ignored",
        );
        return DirectAnswerPreflight::DirectAnswer;
    }
    if route_has_executionless_direct_downgrade(route)
        && decision == DirectAnswerGateDecision::PlannerExecute
        && !current_request_has_structural_execution_target(current_user_request)
        && current_request_resolves_structural_workspace_child_locator_surface(
            state,
            current_user_request,
        )
        .is_none()
        && !has_structural_session_alias_target
        && !force_package_manager_install_preview_execution
    {
        append_route_reason(route, "direct_answer_gate_executionless_promotion_blocked");
        return DirectAnswerPreflight::Clarify(String::new());
    }
    if existing_observed_result_should_stay_chat {
        append_route_reason(route, "direct_answer_gate_existing_observed_result_ignored");
        return DirectAnswerPreflight::DirectAnswer;
    }
    if decision != DirectAnswerGateDecision::PlannerExecute
        && direct_answer_gate_candidate_needs_unbound_context_clarify(
            state,
            current_user_request,
            route,
            &gate,
            auto_locator_path,
            has_authoritative_deictic_anchor,
            has_structural_session_alias_target,
            normalizer_candidate_matches_bound_context,
        )
    {
        return apply_direct_answer_gate_unbound_deictic_clarify(route, &gate);
    }
    match decision {
        DirectAnswerGateDecision::DirectAnswer => {
            let fallback_contract = route.output_contract.clone();
            let resolved_prompt = route.resolved_intent.clone();
            let mut contract = output_contract_from_direct_answer_gate(
                gate.output_contract.clone(),
                &fallback_contract,
            );
            preserve_structured_scalar_extraction_contract(
                &mut contract,
                structured_scalar_extraction,
            );
            if force_inline_transform_execution {
                contract.requires_content_evidence = true;
                contract.locator_kind = crate::OutputLocatorKind::None;
                contract.locator_hint.clear();
                contract.semantic_kind = crate::OutputSemanticKind::None;
                if matches!(
                    contract.response_shape,
                    crate::OutputResponseShape::Free | crate::OutputResponseShape::OneSentence
                ) {
                    contract.response_shape = crate::OutputResponseShape::Strict;
                }
                return promote_direct_answer_gate_to_planner(
                    ctx,
                    &gate,
                    contract,
                    "direct_answer_gate_inline_transform_execute",
                );
            }
            if force_package_manager_detect_execution {
                contract.requires_content_evidence = true;
                contract.locator_kind = crate::OutputLocatorKind::None;
                contract.locator_hint.clear();
                contract.semantic_kind = crate::OutputSemanticKind::PackageManagerDetection;
                if matches!(
                    contract.response_shape,
                    crate::OutputResponseShape::Free | crate::OutputResponseShape::OneSentence
                ) {
                    contract.response_shape = crate::OutputResponseShape::Strict;
                }
                return promote_direct_answer_gate_to_planner(
                    ctx,
                    &gate,
                    contract,
                    "direct_answer_gate_package_manager_detect_execute",
                );
            }
            if normalizer_answer_candidate_from_resolved_prompt(&resolved_prompt).is_some()
                && !output_contract_requires_planner_execution(&contract)
                && bound_direct_answer_candidate_satisfies_output_contract(&contract)
                && matches!(
                    contract.response_shape,
                    crate::OutputResponseShape::OneSentence
                )
                && contract.exact_sentence_count == Some(1)
            {
                append_route_reason(
                    route,
                    "direct_answer_gate_exact_candidate_ignored_execution",
                );
                return DirectAnswerPreflight::DirectAnswer;
            }
            let promoted_workspace_child_context =
                direct_answer_gate_promotes_workspace_child_context(
                    state,
                    current_user_request,
                    route,
                    &mut contract,
                );
            let promoted_artifact_listing =
                promote_artifact_listing_candidate_contract(&resolved_prompt, &mut contract);
            let promoted_recent_file_context =
                direct_answer_gate_should_force_recent_file_context_execution(
                    current_user_request,
                    &resolved_prompt,
                    &contract,
                    recent_request_file_target_count,
                );
            if promoted_workspace_child_context {
                gate.resolved_user_intent = current_user_request.trim().to_string();
            }
            if promoted_recent_file_context {
                contract.requires_content_evidence = true;
                if matches!(contract.locator_kind, crate::OutputLocatorKind::None) {
                    contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
                }
                if matches!(contract.semantic_kind, crate::OutputSemanticKind::None) {
                    contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
                }
            }
            if normalizer_candidate_matches_bound_context
                && bound_direct_answer_candidate_satisfies_output_contract(&contract)
            {
                append_route_reason(route, "direct_answer_gate_bound_candidate_evidence");
                return DirectAnswerPreflight::DirectAnswer;
            }
            if direct_answer_gate_has_recent_count_selection(&gate)
                && direct_answer_gate_recent_count_contract_can_stay_direct(
                    current_user_request,
                    &contract,
                )
            {
                route.output_contract = contract;
                if !gate.resolved_user_intent.trim().is_empty() {
                    route.resolved_intent = gate.resolved_user_intent.trim().to_string();
                }
                append_route_reason(route, "direct_answer_gate_recent_count_selection");
                return DirectAnswerPreflight::DirectAnswer;
            }
            if output_contract_requires_planner_execution(&contract) {
                if existing_observed_result_should_stay_chat {
                    append_route_reason(
                        route,
                        "direct_answer_gate_existing_observed_result_ignored",
                    );
                    return DirectAnswerPreflight::DirectAnswer;
                }
                if direct_answer_gate_untrusted_locator_hint_requires_clarify(
                    state,
                    current_user_request,
                    &contract,
                    &gate.reference_resolution,
                    auto_locator_path,
                    has_authoritative_deictic_anchor,
                    has_structural_session_alias_target,
                ) {
                    return apply_direct_answer_gate_unbound_deictic_clarify(route, &gate);
                }
                if direct_answer_gate_chat_promotion_lacks_structured_target(
                    state,
                    current_user_request,
                    route,
                    &contract,
                    &gate.reference_resolution,
                    has_structural_session_alias_target,
                ) {
                    append_route_reason(
                        route,
                        "direct_answer_gate_chat_promotion_without_structured_target_ignored",
                    );
                    return DirectAnswerPreflight::DirectAnswer;
                }
                if direct_answer_gate_preference_or_memory_promotion_should_stay_chat(
                    state,
                    current_user_request,
                    route,
                    &contract,
                    &gate.reference_resolution,
                    turn_type,
                    has_structural_session_alias_target,
                ) {
                    append_route_reason(
                        route,
                        "direct_answer_gate_preference_memory_context_ignored",
                    );
                    return DirectAnswerPreflight::DirectAnswer;
                }
                if direct_answer_gate_sanitized_freeform_promotion_should_stay_chat(
                    state,
                    current_user_request,
                    route,
                    &contract,
                    &gate.reference_resolution,
                    has_structural_session_alias_target,
                ) {
                    append_route_reason(
                        route,
                        "direct_answer_gate_sanitized_freeform_promotion_ignored",
                    );
                    return DirectAnswerPreflight::DirectAnswer;
                }
                if direct_answer_gate_promotion_depends_only_on_background_context(
                    state,
                    current_user_request,
                    route,
                    &contract,
                    &gate.reference_resolution,
                    has_structural_session_alias_target,
                ) {
                    append_route_reason(route, "direct_answer_gate_background_only_ignored");
                    return DirectAnswerPreflight::DirectAnswer;
                }
                bind_direct_answer_gate_contract_locator(
                    state,
                    current_user_request,
                    &gate,
                    &mut contract,
                );
                bind_session_alias_locator_to_contract(
                    structural_session_alias_locator.as_ref(),
                    &mut contract,
                );
                if direct_answer_gate_untrusted_locator_hint_requires_clarify(
                    state,
                    current_user_request,
                    &contract,
                    &gate.reference_resolution,
                    auto_locator_path,
                    has_authoritative_deictic_anchor,
                    has_structural_session_alias_target,
                ) {
                    return apply_direct_answer_gate_unbound_deictic_clarify(route, &gate);
                }
                if direct_answer_gate_delivery_needs_unbound_existing_file_clarify(
                    state,
                    current_user_request,
                    &contract,
                    auto_locator_path,
                    has_authoritative_deictic_anchor,
                    has_structural_session_alias_target,
                ) {
                    return apply_direct_answer_gate_unbound_deictic_clarify(route, &gate);
                }
                if direct_answer_gate_planner_needs_unbound_locator_clarify(
                    state,
                    current_user_request,
                    &contract,
                    &gate.reference_resolution,
                    auto_locator_path,
                    has_authoritative_deictic_anchor,
                ) {
                    return apply_direct_answer_gate_unbound_deictic_clarify(route, &gate);
                }
                if direct_answer_gate_promotion_needs_unbound_deictic_clarify(
                    state,
                    current_user_request,
                    auto_locator_path,
                    has_authoritative_deictic_anchor,
                    has_structural_session_alias_target,
                    &contract,
                    &gate.reference_resolution,
                ) {
                    return apply_direct_answer_gate_unbound_deictic_clarify(route, &gate);
                }
                let reason_tag = if promoted_recent_file_context {
                    "direct_answer_gate_recent_file_context_execute"
                } else if promoted_artifact_listing {
                    "direct_answer_gate_artifact_listing_execute"
                } else if promoted_workspace_child_context {
                    "direct_answer_gate_workspace_child_context_execute"
                } else if direct_answer_gate_contextual_inline_structured_payload_execute(
                    current_user_request,
                    &contract,
                ) {
                    "inline_structured_payload_context_execute"
                } else {
                    "direct_answer_gate_contract_execute"
                };
                promote_direct_answer_gate_to_planner(ctx, &gate, contract, reason_tag)
            } else {
                DirectAnswerPreflight::DirectAnswer
            }
        }
        DirectAnswerGateDecision::Clarify => {
            if direct_answer_gate_can_skip_for_standalone_freeform_repair(Some(route)) {
                append_route_reason(
                    route,
                    "direct_answer_gate_standalone_freeform_clarify_ignored",
                );
                return DirectAnswerPreflight::DirectAnswer;
            }
            let question = gate.clarify_question.trim();
            if question.is_empty() {
                DirectAnswerPreflight::DirectAnswer
            } else {
                route.set_clarify_gate();
                route.needs_clarify = true;
                route.clarify_question = question.to_string();
                append_route_reason(route, "direct_answer_gate_clarify");
                DirectAnswerPreflight::Clarify(question.to_string())
            }
        }
        DirectAnswerGateDecision::PlannerExecute => {
            let fallback_contract = route.output_contract.clone();
            let mut contract = output_contract_from_direct_answer_gate(
                gate.output_contract.clone(),
                &fallback_contract,
            );
            preserve_structured_scalar_extraction_contract(
                &mut contract,
                structured_scalar_extraction,
            );
            let promoted_workspace_child_context =
                direct_answer_gate_promotes_workspace_child_context(
                    state,
                    current_user_request,
                    route,
                    &mut contract,
                );
            if promoted_workspace_child_context {
                gate.resolved_user_intent = current_user_request.trim().to_string();
            }
            if force_inline_transform_execution {
                contract.requires_content_evidence = true;
                contract.locator_kind = crate::OutputLocatorKind::None;
                contract.locator_hint.clear();
                contract.semantic_kind = crate::OutputSemanticKind::None;
                if matches!(
                    contract.response_shape,
                    crate::OutputResponseShape::Free | crate::OutputResponseShape::OneSentence
                ) {
                    contract.response_shape = crate::OutputResponseShape::Strict;
                }
                return promote_direct_answer_gate_to_planner(
                    ctx,
                    &gate,
                    contract,
                    "direct_answer_gate_inline_transform_execute",
                );
            }
            if existing_observed_result_should_stay_chat {
                append_route_reason(route, "direct_answer_gate_existing_observed_result_ignored");
                return DirectAnswerPreflight::DirectAnswer;
            }
            if direct_answer_gate_untrusted_locator_hint_requires_clarify(
                state,
                current_user_request,
                &contract,
                &gate.reference_resolution,
                auto_locator_path,
                has_authoritative_deictic_anchor,
                has_structural_session_alias_target,
            ) {
                return apply_direct_answer_gate_unbound_deictic_clarify(route, &gate);
            }
            if direct_answer_gate_chat_promotion_lacks_structured_target(
                state,
                current_user_request,
                route,
                &contract,
                &gate.reference_resolution,
                has_structural_session_alias_target,
            ) {
                append_route_reason(
                    route,
                    "direct_answer_gate_chat_promotion_without_structured_target_ignored",
                );
                return DirectAnswerPreflight::DirectAnswer;
            }
            if direct_answer_gate_preference_or_memory_promotion_should_stay_chat(
                state,
                current_user_request,
                route,
                &contract,
                &gate.reference_resolution,
                turn_type,
                has_structural_session_alias_target,
            ) {
                append_route_reason(
                    route,
                    "direct_answer_gate_preference_memory_context_ignored",
                );
                return DirectAnswerPreflight::DirectAnswer;
            }
            if direct_answer_gate_sanitized_freeform_promotion_should_stay_chat(
                state,
                current_user_request,
                route,
                &contract,
                &gate.reference_resolution,
                has_structural_session_alias_target,
            ) {
                append_route_reason(
                    route,
                    "direct_answer_gate_sanitized_freeform_promotion_ignored",
                );
                return DirectAnswerPreflight::DirectAnswer;
            }
            if normalizer_candidate_matches_bound_context
                && bound_direct_answer_candidate_satisfies_output_contract(&contract)
            {
                append_route_reason(route, "direct_answer_gate_bound_candidate_evidence");
                return DirectAnswerPreflight::DirectAnswer;
            }
            if direct_answer_gate_promotion_depends_only_on_background_context(
                state,
                current_user_request,
                route,
                &contract,
                &gate.reference_resolution,
                has_structural_session_alias_target,
            ) {
                append_route_reason(route, "direct_answer_gate_background_only_ignored");
                return DirectAnswerPreflight::DirectAnswer;
            }
            bind_direct_answer_gate_contract_locator(
                state,
                current_user_request,
                &gate,
                &mut contract,
            );
            bind_session_alias_locator_to_contract(
                structural_session_alias_locator.as_ref(),
                &mut contract,
            );
            if direct_answer_gate_untrusted_locator_hint_requires_clarify(
                state,
                current_user_request,
                &contract,
                &gate.reference_resolution,
                auto_locator_path,
                has_authoritative_deictic_anchor,
                has_structural_session_alias_target,
            ) {
                return apply_direct_answer_gate_unbound_deictic_clarify(route, &gate);
            }
            if direct_answer_gate_delivery_needs_unbound_existing_file_clarify(
                state,
                current_user_request,
                &contract,
                auto_locator_path,
                has_authoritative_deictic_anchor,
                has_structural_session_alias_target,
            ) {
                return apply_direct_answer_gate_unbound_deictic_clarify(route, &gate);
            }
            if direct_answer_gate_planner_needs_unbound_locator_clarify(
                state,
                current_user_request,
                &contract,
                &gate.reference_resolution,
                auto_locator_path,
                has_authoritative_deictic_anchor,
            ) {
                return apply_direct_answer_gate_unbound_deictic_clarify(route, &gate);
            }
            if direct_answer_gate_promotion_needs_unbound_deictic_clarify(
                state,
                current_user_request,
                auto_locator_path,
                has_authoritative_deictic_anchor,
                has_structural_session_alias_target,
                &contract,
                &gate.reference_resolution,
            ) {
                return apply_direct_answer_gate_unbound_deictic_clarify(route, &gate);
            }
            let reason_tag = if promoted_workspace_child_context {
                "direct_answer_gate_workspace_child_context_execute"
            } else if direct_answer_gate_contextual_inline_structured_payload_execute(
                current_user_request,
                &contract,
            ) {
                "inline_structured_payload_context_execute"
            } else {
                "direct_answer_gate_execute"
            };
            promote_direct_answer_gate_to_planner(ctx, &gate, contract, reason_tag)
        }
    }
}

pub(super) fn apply_direct_answer_gate_unbound_deictic_clarify(
    route: &mut crate::RouteResult,
    gate: &DirectAnswerGateOut,
) -> DirectAnswerPreflight {
    let mut preserved_contract = output_contract_from_direct_answer_gate(
        gate.output_contract.clone(),
        &route.output_contract,
    );
    preserved_contract.locator_kind = crate::OutputLocatorKind::None;
    preserved_contract.locator_hint.clear();

    route.set_clarify_gate();
    route.needs_clarify = true;
    route.clarify_question.clear();
    route.wants_file_delivery = preserved_contract.delivery_required
        || matches!(
            preserved_contract.response_shape,
            crate::OutputResponseShape::FileToken
        );
    route.output_contract = preserved_contract;
    append_route_reason(route, "direct_answer_gate_unbound_deictic_clarify");
    DirectAnswerPreflight::Clarify(route.clarify_question.clone())
}

pub(super) fn direct_answer_gate_route_context(
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> String {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
        return "<none>".to_string();
    };
    let mut lines = Vec::new();
    let (resolved_intent, removed_answer_candidate) =
        strip_embedded_answer_candidate_from_intent(route.resolved_intent.trim());
    if !resolved_intent.is_empty() {
        lines.push(format!("resolved_user_intent: {resolved_intent}"));
    }
    if removed_answer_candidate {
        lines.push("normalizer_answer_candidate_present: true (not runtime evidence)".to_string());
    }
    let locator_hint = route.output_contract.locator_hint.trim();
    if !locator_hint.is_empty() {
        lines.push(format!("locator_hint: {locator_hint}"));
    }
    lines.push(format!(
        "response_shape: {}",
        route.output_contract.response_shape.as_str()
    ));
    lines.push(format!(
        "semantic_kind: {}",
        route.output_contract.semantic_kind.as_str()
    ));
    lines.push(format!(
        "requires_content_evidence: {}",
        route.output_contract.requires_content_evidence
    ));
    lines.push(format!(
        "delivery_required: {}",
        route.output_contract.delivery_required
    ));
    let route_reason = route.route_reason.trim();
    if !route_reason.is_empty() {
        lines.push(format!("prior_route_reason: {route_reason}"));
    }
    format!(
        "### PRIOR_ROUTE_CONTEXT\nReview this prior route context, but do not treat it as observed evidence. The current request and runtime-evidence rules win over prior answer candidates or prior route reasons.\n{}\n",
        lines.join("\n")
    )
}

pub(super) fn direct_answer_gate_recent_execution_context(
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> String {
    let Some(context) = agent_run_context
        .and_then(|ctx| ctx.cross_turn_recent_execution_context.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "<none>")
    else {
        return "<none>".to_string();
    };
    let context = crate::providers::utf8_safe_prefix(context, 6000);
    format!(
        "### RECENT_EXECUTION_CONTEXT\nUse this only for current-turn follow-up reference binding. Previous executed targets are authoritative for relative/ordinal file or action references. Paths mentioned inside a prior file excerpt are content, not the executed file target unless the current request explicitly asks about the excerpt content.\n{context}"
    )
}

pub(super) fn direct_answer_gate_runtime_context(state: &AppState) -> String {
    let current_process_cwd = std::env::current_dir()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| "<unknown>".to_string());
    format!(
        "workspace_root: {}\ncurrent_process_cwd: {}\nruntime_has_tools: true",
        state.skill_rt.workspace_root.display(),
        current_process_cwd
    )
}

pub(super) async fn run_direct_answer_gate(
    state: &AppState,
    task: &ClaimedTask,
    user_request: &str,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> Option<DirectAnswerGateOut> {
    let resolved = match crate::bootstrap::load_required_prompt_template_for_state_with_meta(
        state,
        DIRECT_ANSWER_GATE_PROMPT_LOGICAL_PATH,
    ) {
        Ok(resolved) => resolved,
        Err(err) => {
            tracing::info!(
                "{} direct_answer_gate prompt_missing task_id={} err={}",
                crate::highlight_tag("routing"),
                task.task_id,
                err
            );
            return None;
        }
    };
    let route_context = direct_answer_gate_route_context(agent_run_context);
    let recent_execution_context = direct_answer_gate_recent_execution_context(agent_run_context);
    let runtime_context = direct_answer_gate_runtime_context(state);
    let prompt = crate::render_prompt_template(
        &resolved.template,
        &[
            ("__REQUEST__", user_request.trim()),
            ("__ROUTE_CONTEXT__", &route_context),
            ("__RECENT_EXECUTION_CONTEXT__", &recent_execution_context),
            ("__RUNTIME_CONTEXT__", &runtime_context),
        ],
    );
    crate::log_prompt_render_with_version(
        state,
        &task.task_id,
        "direct_answer_gate_prompt",
        &resolved.source,
        resolved.version.as_deref(),
        None,
    );
    let prompt_source = resolved.source;
    let llm_out = match crate::llm_gateway::run_with_fallback_with_prompt_source(
        state,
        task,
        &prompt,
        &prompt_source,
    )
    .await
    {
        Ok(value) => value,
        Err(err) => {
            tracing::info!(
                "{} direct_answer_gate llm_failed task_id={} err={}",
                crate::highlight_tag("routing"),
                task.task_id,
                err
            );
            return None;
        }
    };
    match crate::prompt_utils::validate_against_schema::<DirectAnswerGateOut>(
        &llm_out,
        crate::prompt_utils::PromptSchemaId::DirectAnswerGate,
    ) {
        Ok(validated) => Some(validated.value),
        Err(err) => {
            tracing::info!(
                "{} direct_answer_gate schema_validation_failed task_id={} err={}",
                crate::highlight_tag("routing"),
                task.task_id,
                err
            );
            None
        }
    }
}

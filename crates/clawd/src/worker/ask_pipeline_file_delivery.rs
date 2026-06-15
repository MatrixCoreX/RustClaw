pub(super) fn direct_existing_file_delivery_token(
    route_result: &crate::RouteResult,
) -> Option<String> {
    let contract = &route_result.output_contract;
    if route_result.needs_clarify
        || !contract.delivery_required
        || contract.response_shape != crate::OutputResponseShape::FileToken
        || contract.delivery_intent != crate::OutputDeliveryIntent::FileSingle
        || contract.locator_kind != crate::OutputLocatorKind::Path
    {
        return None;
    }
    if !matches!(
        contract.semantic_kind,
        crate::OutputSemanticKind::None | crate::OutputSemanticKind::GeneratedFileDelivery
    ) {
        return None;
    }
    if contract.semantic_kind == crate::OutputSemanticKind::GeneratedFileDelivery
        && route_result
            .route_reason
            .split(';')
            .map(str::trim)
            .any(|part| part == "generated_file_delivery_allows_runtime_target")
    {
        return None;
    }
    let path = contract.locator_hint.trim();
    if path.is_empty() {
        return None;
    }
    let path = std::path::Path::new(path);
    if !path.is_file() {
        return None;
    }
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    Some(format!("FILE:{}", path.display()))
}

pub(super) fn generated_file_delivery_uses_runtime_target(
    route_result: &crate::RouteResult,
) -> bool {
    route_result.output_contract.semantic_kind == crate::OutputSemanticKind::GeneratedFileDelivery
        && super::route_reason_has_marker(
            route_result,
            "generated_file_delivery_allows_runtime_target",
        )
}

pub(super) fn prebind_direct_file_delivery_locator_before_deictic_guard(
    state: &crate::AppState,
    recent_execution_context: &str,
    route_result: &mut crate::RouteResult,
) -> bool {
    if route_result.needs_clarify
        || !route_result.output_contract.delivery_required
        || route_result.output_contract.response_shape != crate::OutputResponseShape::FileToken
        || route_result.output_contract.delivery_intent != crate::OutputDeliveryIntent::FileSingle
        || crate::worker::has_explicit_path_or_url_locator_hint(
            route_result.output_contract.locator_hint.trim(),
        )
    {
        return false;
    }
    let Some(path) = super::direct_auto_locator_path(state, route_result, recent_execution_context)
    else {
        return false;
    };
    if super::locator_hint_points_to_workspace_root(&state.skill_rt.workspace_root, &path) {
        if generated_file_delivery_uses_runtime_target(route_result) {
            return false;
        }
        route_result.needs_clarify = true;
        route_result.set_clarify_gate();
        route_result.clarify_question.clear();
        route_result.output_contract.locator_kind = crate::OutputLocatorKind::None;
        route_result.output_contract.locator_hint.clear();
        super::append_route_reason(
            route_result,
            "direct_file_delivery_workspace_root_locator_rejected",
        );
        return false;
    }
    route_result.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route_result.output_contract.locator_hint = path;
    super::append_route_reason(
        route_result,
        "direct_file_delivery_locator_prebound_before_deictic_guard",
    );
    true
}

pub(super) fn prebind_file_delivery_locator_from_recent_ordered_resolved_prompt(
    state: &crate::AppState,
    resolved_prompt: &str,
    recent_execution_context: &str,
    route_result: &mut crate::RouteResult,
) -> bool {
    if !route_result.needs_clarify
        || !route_result.output_contract.delivery_required
        || route_result.output_contract.response_shape != crate::OutputResponseShape::FileToken
        || route_result.output_contract.delivery_intent != crate::OutputDeliveryIntent::FileSingle
        || route_result.output_contract.locator_kind != crate::OutputLocatorKind::None
        || !route_result.output_contract.locator_hint.trim().is_empty()
    {
        return false;
    }
    let Some(path) = super::resolve_recent_ordered_entry_target_from_resolved_prompt(
        state,
        resolved_prompt,
        recent_execution_context,
    ) else {
        return false;
    };
    route_result.wants_file_delivery = true;
    super::promote_clarify_observation_to_execute_with_locator(
        route_result,
        crate::OutputLocatorKind::Path,
        path,
        "file_delivery_locator_prebound_from_recent_ordered_resolved_prompt",
    )
}

pub(super) fn prebind_file_delivery_locator_from_resolved_prompt_path(
    state: &crate::AppState,
    resolved_prompt: &str,
    route_result: &mut crate::RouteResult,
) -> bool {
    if !route_result.output_contract.delivery_required
        || route_result.output_contract.response_shape != crate::OutputResponseShape::FileToken
        || route_result.output_contract.delivery_intent != crate::OutputDeliveryIntent::FileSingle
        || route_result.output_contract.locator_kind != crate::OutputLocatorKind::None
        || !route_result.output_contract.locator_hint.trim().is_empty()
    {
        return false;
    }
    let Some(path) = super::resolved_prompt_existing_workspace_locator(state, resolved_prompt)
        .or_else(|| {
            super::resolved_prompt_existing_workspace_locator(state, &route_result.resolved_intent)
        })
        .filter(|path| std::path::Path::new(path).is_file())
    else {
        return false;
    };
    route_result.wants_file_delivery = true;
    if route_result.needs_clarify || route_result.is_chat_gate() {
        return super::promote_clarify_observation_to_execute_with_locator(
            route_result,
            crate::OutputLocatorKind::Path,
            path,
            "file_delivery_locator_prebound_from_resolved_prompt_path",
        );
    }
    route_result.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route_result.output_contract.locator_hint = path;
    super::append_route_reason(
        route_result,
        "file_delivery_locator_prebound_from_resolved_prompt_path",
    );
    true
}

fn route_requires_existing_file_delivery(route_result: &crate::RouteResult) -> bool {
    if matches!(
        route_result.output_contract.semantic_kind,
        crate::OutputSemanticKind::GeneratedFileDelivery
    ) {
        return false;
    }
    route_result.wants_file_delivery
        || route_result.output_contract.delivery_required
        || matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::FileToken
        )
        || matches!(
            route_result.output_contract.delivery_intent,
            crate::OutputDeliveryIntent::FileSingle
        )
}

fn current_request_has_file_delivery_locator_binding(
    state: &crate::AppState,
    prompt: &str,
) -> bool {
    super::current_request_has_concrete_locator_surface(prompt)
        || super::current_request_resolves_workspace_child_locator(state, prompt).is_some()
}

pub(super) fn unbound_existing_file_delivery_route_should_force_clarify(
    state: &crate::AppState,
    prompt: &str,
    route_result: &crate::RouteResult,
    has_authoritative_deictic_anchor: bool,
) -> bool {
    if route_result.needs_clarify
        || has_authoritative_deictic_anchor
        || super::route_reason_has_marker(
            route_result,
            "file_delivery_locator_prebound_from_resolved_prompt_path",
        )
        || super::route_reason_has_marker(
            route_result,
            "file_delivery_locator_prebound_from_recent_ordered_resolved_prompt",
        )
        || super::route_reason_has_marker(
            route_result,
            "direct_file_delivery_locator_prebound_before_deictic_guard",
        )
        || !route_requires_existing_file_delivery(route_result)
        || current_request_has_file_delivery_locator_binding(state, prompt)
    {
        return false;
    }
    true
}

pub(super) fn directory_file_delivery_without_structured_selection_should_force_clarify(
    state: &crate::AppState,
    prompt: &str,
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    if route_result.needs_clarify
        || !route_result.is_execute_gate()
        || !route_result.output_contract.delivery_required
        || route_result.output_contract.response_shape != crate::OutputResponseShape::FileToken
        || route_result.output_contract.delivery_intent != crate::OutputDeliveryIntent::FileSingle
        || crate::worker::has_explicit_path_or_url_locator_hint(prompt)
        || super::current_request_has_self_contained_structured_payload(prompt)
        || super::state_patch_allows_deictic_locator_guard_bypass(turn_analysis)
        || state_patch_has_ordered_entry_reference(turn_analysis)
        || turn_analysis_reuses_active_target(turn_analysis)
        || route_repaired_active_task_binding(route_result)
        || session_alias_binding_matches_prompt(prompt, session_snapshot)
        || !route_locator_hint_resolves_to_existing_directory(state, route_result)
    {
        return false;
    }
    true
}

fn state_patch_has_ordered_entry_reference(
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    turn_analysis
        .and_then(|analysis| analysis.state_patch.as_ref())
        .is_some_and(|state_patch| {
            state_patch.get("ordered_entry_ref").is_some()
                || state_patch.get("ordered_entry_reference").is_some()
        })
}

fn turn_analysis_reuses_active_target(
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    turn_analysis.is_some_and(|analysis| {
        matches!(
            analysis.target_task_policy,
            Some(crate::intent_router::TargetTaskPolicy::ReuseActive)
        )
    })
}

fn route_locator_hint_resolves_to_existing_directory(
    state: &crate::AppState,
    route_result: &crate::RouteResult,
) -> bool {
    if route_result.output_contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace {
        return state.skill_rt.workspace_root.is_dir();
    }
    if !matches!(
        route_result.output_contract.locator_kind,
        crate::OutputLocatorKind::Path | crate::OutputLocatorKind::Filename
    ) {
        return false;
    }
    super::resolve_existing_workspace_locator_hint(
        state,
        route_result.output_contract.locator_hint.trim(),
    )
    .is_some_and(|path| std::path::Path::new(&path).is_dir())
}

pub(super) fn active_anchor_file_delivery_without_structured_reference_should_force_clarify(
    prompt: &str,
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    if route_result.needs_clarify
        || !route_result.is_execute_gate()
        || !route_requires_existing_file_delivery(route_result)
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.locator_hint.trim().is_empty()
        || !matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::Path | crate::OutputLocatorKind::Filename
        )
        || crate::worker::has_explicit_path_or_url_locator_hint(prompt)
        || super::locator_hint_full_file_name_token_present_in_prompt(
            prompt,
            route_result.output_contract.locator_hint.trim(),
        )
        || super::current_request_has_self_contained_structured_payload(prompt)
        || super::state_patch_allows_deictic_locator_guard_bypass(turn_analysis)
        || state_patch_has_ordered_entry_reference(turn_analysis)
        || turn_analysis_reuses_active_target(turn_analysis)
        || route_repaired_active_task_binding(route_result)
        || session_alias_binding_matches_prompt(prompt, session_snapshot)
    {
        return false;
    }
    super::followup_frame_has_matching_target(
        session_snapshot.active_followup_frame.as_ref(),
        route_result,
    ) || super::observed_facts_have_matching_target(
        session_snapshot.active_observed_facts.as_ref(),
        route_result,
    )
}

fn route_repaired_active_task_binding(route_result: &crate::RouteResult) -> bool {
    route_result
        .route_reason
        .split(';')
        .map(str::trim)
        .any(|part| {
            part.starts_with("llm_semantic_contract_repair:active_task_invalid_turn_binding")
        })
}

fn session_alias_binding_matches_prompt(
    prompt: &str,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    session_snapshot
        .conversation_state
        .as_ref()
        .is_some_and(|state| {
            state.alias_bindings.iter().any(|binding| {
                let alias = binding.alias.trim();
                !alias.is_empty()
                    && crate::conversation_state::alias_surface_matches_prompt(prompt, alias)
            })
        })
}

#[cfg(test)]
#[path = "ask_pipeline_file_delivery_tests.rs"]
mod tests;

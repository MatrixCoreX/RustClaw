pub(super) fn post_route_promote_resolved_multifile_targets_to_execute(
    state: &crate::AppState,
    prompt: &str,
    resolved_prompt: &str,
    post_route: &mut crate::post_route_policy::PostRoutePolicyResult,
) -> bool {
    let query = format!(
        "{}\n{}\n{}",
        prompt, resolved_prompt, post_route.execution_route_result.resolved_intent
    );
    if !super::promote_clarify_resolved_multifile_targets_to_execute(
        state,
        &query,
        &mut post_route.execution_route_result,
    ) {
        return false;
    }
    post_route.missing_locator_for_path_scoped_content = false;
    post_route.auto_locator_path = None;
    post_route.auto_locator_hint = None;
    post_route.auto_locator_resolved_direct = false;
    post_route.fuzzy_locator_suggestions.clear();
    post_route.clarify_reason.clear();
    post_route.clarify_reason_kind = crate::post_route_policy::ClarifyReasonKind::RouteReasonText;
    post_route.gate_record = crate::post_route_policy::PostRouteGateRecord::new(
        "post_route_resolved_multifile_targets_promoted_to_execute",
        crate::post_route_policy::PostRoutePolicyOutcome::Execute,
    );
    super::append_route_reason(
        &mut post_route.execution_route_result,
        "post_route_resolved_multifile_targets_promoted_to_execute",
    );
    true
}

pub(super) fn direct_auto_locator_path(
    state: &crate::AppState,
    route_result: &crate::RouteResult,
    recent_execution_context: &str,
) -> Option<String> {
    if !super::should_attempt_auto_locator(route_result) {
        return None;
    }
    if let Some(crate::post_route_policy::LocatorResolution::Direct(path)) =
        super::current_workspace_locator_resolution(&state.skill_rt.workspace_root, route_result)
    {
        return Some(path);
    }
    let locator_hint = route_result.output_contract.locator_hint.trim();
    if locator_hint.is_empty() {
        return None;
    }
    let locator_kind = super::effective_auto_locator_kind(route_result);
    super::super::try_resolve_implicit_locator_path(
        state,
        locator_hint,
        locator_hint,
        locator_kind,
        Some(recent_execution_context),
    )
    .and_then(|resolution| match resolution {
        super::super::LocatorAutoResolution::Direct(path) => Some(path),
        super::super::LocatorAutoResolution::Fuzzy(_) => None,
    })
}

pub(super) fn auto_locator_scalar_file_without_current_locator_should_force_clarify(
    state: &crate::AppState,
    prompt: &str,
    route_result: &crate::RouteResult,
    auto_locator_path: Option<&str>,
) -> bool {
    let Some(auto_locator_path) = auto_locator_path
        .map(str::trim)
        .filter(|path| !path.is_empty())
    else {
        return false;
    };
    if !std::path::Path::new(auto_locator_path).is_file()
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || route_result.wants_file_delivery
        || route_result.output_contract.response_shape != crate::OutputResponseShape::Scalar
        || route_result.output_contract.semantic_kind != crate::OutputSemanticKind::None
        || !matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::Path
                | crate::OutputLocatorKind::Filename
                | crate::OutputLocatorKind::CurrentWorkspace
        )
    {
        return false;
    }
    if super::route_reason_has_marker(
        route_result,
        super::SESSION_ALIAS_LOCATOR_PREBOUND_FROM_CURRENT_REQUEST,
    ) {
        return false;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    !super::structured_field_route_has_current_locator_surface(state, &surface)
}

#[cfg(test)]
#[path = "ask_pipeline_post_route_binding_tests.rs"]
mod tests;

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
        || !route_has_structured_scalar_field_contract(route_result)
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

fn route_has_structured_scalar_field_contract(route_result: &crate::RouteResult) -> bool {
    route_result
        .output_contract
        .self_extension
        .structured_field_selector
        .as_deref()
        .map(str::trim)
        .is_some_and(|selector| !selector.is_empty())
        || super::auto_locator_binding::route_reason_has_structured_field_selector_marker(
            route_result,
        )
}

#[cfg(test)]
#[path = "ask_pipeline_post_route_binding_tests.rs"]
mod tests;

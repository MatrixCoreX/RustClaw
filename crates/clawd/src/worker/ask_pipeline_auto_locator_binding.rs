pub(super) fn bind_structured_field_read_to_auto_locator(
    post_route: &mut crate::post_route_policy::PostRoutePolicyResult,
) -> bool {
    let selector_present = post_route
        .execution_route_result
        .output_contract
        .self_extension
        .structured_field_selector
        .as_deref()
        .map(str::trim)
        .is_some_and(|selector| !selector.is_empty());
    if !selector_present
        || post_route
            .execution_route_result
            .output_contract
            .response_shape
            != crate::OutputResponseShape::Scalar
        || !post_route
            .execution_route_result
            .output_contract
            .requires_content_evidence
    {
        return false;
    }
    let Some(path) = post_route
        .auto_locator_path
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
    else {
        return false;
    };
    if !std::path::Path::new(path).is_file() {
        return false;
    }
    let contract = &mut post_route.execution_route_result.output_contract;
    if contract.locator_kind == crate::OutputLocatorKind::Path
        && contract.locator_hint.trim() == path
    {
        return false;
    }
    contract.locator_kind = crate::OutputLocatorKind::Path;
    contract.locator_hint = path.to_string();
    super::append_route_reason(
        &mut post_route.execution_route_result,
        "structured_field_read_bound_to_auto_locator",
    );
    true
}

pub(super) fn route_reason_has_structured_field_selector_marker(
    route_result: &crate::RouteResult,
) -> bool {
    super::route_reason_has_marker(
        route_result,
        "structured_field_selector_requires_scalar_value",
    ) || super::route_reason_has_marker(
        route_result,
        "structured_keys_scalar_response_requires_field_value",
    ) || super::route_reason_has_marker(
        route_result,
        "config_validation_field_selector_requires_scalar_value",
    ) || super::route_reason_has_marker(
        route_result,
        "single_path_config_field_extraction_contract_semantically_valid",
    ) || super::route_reason_has_marker(
        route_result,
        "llm_semantic_contract_repair:single_path_config_field_extraction_contract_semantically_valid",
    )
}

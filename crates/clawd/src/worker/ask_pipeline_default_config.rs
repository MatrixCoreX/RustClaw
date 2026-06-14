use crate::AppState;

const RUSTCLAW_MAIN_CONFIG_LOGICAL_PATH: &str = "configs/config.toml";

fn route_is_default_main_config_contract(route: &crate::RouteResult) -> bool {
    !route.wants_file_delivery
        && !route.output_contract.delivery_required
        && route.schedule_kind == crate::ScheduleKind::None
        && route.output_contract.delivery_intent == crate::OutputDeliveryIntent::None
        && route.output_contract.requires_content_evidence
        && matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::ConfigRiskAssessment
                | crate::OutputSemanticKind::ConfigValidation
        )
}

fn bind_default_main_config_contract_route(
    route: &mut crate::RouteResult,
    reason_code: &'static str,
) {
    route.needs_clarify = false;
    route.clarify_question.clear();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = RUSTCLAW_MAIN_CONFIG_LOGICAL_PATH.to_string();
    route.output_contract.delivery_required = false;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::None;
    route.output_contract.requires_content_evidence = true;
    let finalize = crate::post_route_policy::content_evidence_execution_finalize_style(
        &route.output_contract,
        false,
    )
    .unwrap_or(crate::ActFinalizeStyle::ChatWrapped);
    route.set_planner_execute_finalize(finalize);
    super::append_route_reason(route, reason_code);
}

fn prompt_allows_default_main_config_binding(prompt: &str) -> bool {
    let surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    !surface.has_explicit_path_or_url() && !surface.has_delivery_token_reference()
}

fn locator_hint_is_default_main_config(
    workspace_root: &std::path::Path,
    locator_hint: &str,
) -> bool {
    let hint = locator_hint.trim();
    if hint.is_empty() {
        return false;
    }
    let main_config = workspace_root.join(RUSTCLAW_MAIN_CONFIG_LOGICAL_PATH);
    let hint_path = std::path::Path::new(hint);
    if hint_path == std::path::Path::new(RUSTCLAW_MAIN_CONFIG_LOGICAL_PATH) {
        return true;
    }
    if hint_path.is_absolute() {
        return hint_path == main_config;
    }
    workspace_root.join(hint_path) == main_config
}

fn locator_hint_is_workspace_identity(
    workspace_root: &std::path::Path,
    locator_hint: &str,
) -> bool {
    let Some(workspace_name) = workspace_root.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let Some(hint_name) = std::path::Path::new(locator_hint.trim())
        .file_name()
        .and_then(|name| name.to_str())
    else {
        return false;
    };
    hint_name.eq_ignore_ascii_case(workspace_name)
        || hint_name
            .strip_suffix(".toml")
            .is_some_and(|stem| stem.eq_ignore_ascii_case(workspace_name))
}

fn route_locator_hint_allows_default_main_config_binding(
    state: &AppState,
    prompt: &str,
    route: &crate::RouteResult,
) -> bool {
    let locator_hint = route.output_contract.locator_hint.trim();
    if locator_hint.is_empty()
        || locator_hint_is_default_main_config(&state.skill_rt.workspace_root, locator_hint)
    {
        return true;
    }
    prompt_allows_default_main_config_binding(prompt)
        && locator_hint_is_workspace_identity(&state.skill_rt.workspace_root, locator_hint)
}

pub(super) fn prebind_config_contract_default_main_config_locator(
    state: &AppState,
    prompt: &str,
    route: &mut crate::RouteResult,
) -> bool {
    let default_config_path = state
        .skill_rt
        .workspace_root
        .join(RUSTCLAW_MAIN_CONFIG_LOGICAL_PATH);
    if !default_config_path.is_file()
        || !route_is_default_main_config_contract(route)
        || !route_locator_hint_allows_default_main_config_binding(state, prompt, route)
    {
        return false;
    }
    bind_default_main_config_contract_route(route, "config_contract_default_main_config_prebound");
    true
}

pub(super) fn promote_config_contract_default_main_config_to_execute(
    state: &AppState,
    prompt: &str,
    post_route: &mut crate::post_route_policy::PostRoutePolicyResult,
) -> bool {
    let default_config_path = state
        .skill_rt
        .workspace_root
        .join(RUSTCLAW_MAIN_CONFIG_LOGICAL_PATH);
    if !default_config_path.is_file() {
        return false;
    }
    let route = &mut post_route.execution_route_result;
    let route_can_accept_default_config_locator =
        route.needs_clarify || route.is_clarify_gate() || route.is_execute_gate();
    let locator_hint = route.output_contract.locator_hint.trim();
    let locator_hint_is_empty = route.output_contract.locator_hint.trim().is_empty();
    let locator_hint_is_main_config =
        locator_hint_is_default_main_config(&state.skill_rt.workspace_root, locator_hint);
    let locator_hint_is_workspace_identity = prompt_allows_default_main_config_binding(prompt)
        && locator_hint_is_workspace_identity(&state.skill_rt.workspace_root, locator_hint);
    let locator_hint_is_internal_prebound = super::route_reason_has_marker(
        route,
        "workspace_child_locator_prebound_from_current_request",
    ) || super::route_reason_has_marker(
        route,
        "workspace_locator_hint_prebound_from_current_request",
    ) || super::route_reason_has_marker(
        route,
        "workspace_root_locator_prebound_from_current_request",
    );
    if !route_can_accept_default_config_locator
        || !route_is_default_main_config_contract(route)
        || (!locator_hint_is_empty
            && !locator_hint_is_internal_prebound
            && !locator_hint_is_main_config
            && !locator_hint_is_workspace_identity)
    {
        return false;
    }
    bind_default_main_config_contract_route(
        route,
        "config_contract_default_main_config_to_planner",
    );
    post_route.auto_locator_path = None;
    post_route.auto_locator_hint = None;
    post_route.auto_locator_resolved_direct = false;
    post_route.missing_locator_for_path_scoped_content = false;
    post_route.fuzzy_locator_suggestions.clear();
    post_route.clarify_reason.clear();
    post_route.clarify_reason_kind = crate::post_route_policy::ClarifyReasonKind::RouteReasonText;
    post_route.gate_record = crate::post_route_policy::PostRouteGateRecord::new(
        "post_route_config_contract_default_main_config_to_planner",
        crate::post_route_policy::PostRoutePolicyOutcome::Execute,
    );
    true
}

#[cfg(test)]
#[path = "ask_pipeline_default_config_tests.rs"]
mod tests;

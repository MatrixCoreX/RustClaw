use crate::AppState;
use serde_json::{json, Value};

const RUSTCLAW_MAIN_CONFIG_LOGICAL_PATH: &str = "configs/config.toml";

fn route_is_default_main_config_contract(route: &crate::RouteResult) -> bool {
    !route.wants_file_delivery
        && !route.output_contract.delivery_required
        && route.schedule_kind == crate::ScheduleKind::None
        && route.output_contract.delivery_intent == crate::OutputDeliveryIntent::None
        && route.output_contract.requires_content_evidence
        && route_has_default_main_config_contract_marker(route)
}

fn route_has_default_main_config_contract_marker(route: &crate::RouteResult) -> bool {
    super::route_reason_has_marker(route, "config_validation")
        || super::route_reason_has_marker(route, "config_risk_assessment")
        || super::route_reason_has_marker(route, "rustclaw_main_config_contract")
}

fn prompt_allows_default_main_config_binding(prompt: &str) -> bool {
    let surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    !crate::worker::has_explicit_path_or_url_locator_hint(prompt)
        && !surface.has_delivery_token_reference()
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

fn locator_hint_matches_current_request_workspace_child(
    state: &AppState,
    prompt: &str,
    locator_hint: &str,
) -> bool {
    let Some(current_request_path) =
        super::current_request_resolves_workspace_child_locator(state, prompt)
    else {
        return false;
    };
    let hint = locator_hint.trim();
    if hint.is_empty() {
        return false;
    }
    let hint_path = std::path::Path::new(hint);
    let hint_path = if hint_path.is_absolute() {
        hint_path.to_path_buf()
    } else {
        state.skill_rt.workspace_root.join(hint_path)
    };
    let hint_path = hint_path.canonicalize().unwrap_or(hint_path);
    let current_request_path = std::path::PathBuf::from(current_request_path);
    hint_path == current_request_path
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
    if !prompt_allows_default_main_config_binding(prompt) {
        return false;
    }
    locator_hint_is_workspace_identity(&state.skill_rt.workspace_root, locator_hint)
        || locator_hint_matches_current_request_workspace_child(state, prompt, locator_hint)
}

pub(super) fn default_main_config_contract_observation(
    state: &AppState,
    prompt: &str,
    route: &crate::RouteResult,
) -> Option<Value> {
    let default_config_path = state
        .skill_rt
        .workspace_root
        .join(RUSTCLAW_MAIN_CONFIG_LOGICAL_PATH);
    if !route_is_default_main_config_contract(route)
        || !route_locator_hint_allows_default_main_config_binding(state, prompt, route)
    {
        return None;
    }
    Some(json!({
        "source": "boundary_contract",
        "contract": "rustclaw_main_config",
        "logical_path": RUSTCLAW_MAIN_CONFIG_LOGICAL_PATH,
        "workspace_path": default_config_path.display().to_string(),
        "exists": default_config_path.is_file(),
        "route_markers": default_main_config_contract_markers(route),
    }))
}

fn default_main_config_contract_markers(route: &crate::RouteResult) -> Vec<&'static str> {
    [
        "config_validation",
        "config_risk_assessment",
        "rustclaw_main_config_contract",
    ]
    .into_iter()
    .filter(|marker| super::route_reason_has_marker(route, marker))
    .collect()
}

pub(super) fn defer_config_contract_default_main_config_after_locator_policy(
    state: &AppState,
    prompt: &str,
    post_route: &mut crate::post_route_policy::PostRoutePolicyResult,
) -> bool {
    if default_main_config_contract_observation(state, prompt, &post_route.execution_route_result)
        .is_none()
    {
        return false;
    }
    if prompt_allows_default_main_config_binding(prompt) {
        if post_route.auto_locator_resolved_direct {
            post_route.auto_locator_path = None;
            post_route.auto_locator_hint = None;
            post_route.auto_locator_resolved_direct = false;
            post_route.fuzzy_locator_suggestions.clear();
        }
        post_route
            .execution_route_result
            .output_contract
            .locator_hint
            .clear();
        post_route
            .execution_route_result
            .output_contract
            .locator_kind = crate::OutputLocatorKind::None;
    }
    super::append_route_reason(
        &mut post_route.execution_route_result,
        "config_contract_default_main_config_deferred_to_loop",
    );
    true
}

#[cfg(test)]
#[path = "ask_pipeline_default_config_tests.rs"]
mod tests;

pub(super) fn effective_auto_locator_kind(
    route_result: &crate::RouteResult,
) -> crate::OutputLocatorKind {
    if route_result.output_contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace
        && !route_result.output_contract.locator_hint.trim().is_empty()
    {
        crate::OutputLocatorKind::Path
    } else {
        route_result.output_contract.locator_kind
    }
}

pub(super) fn current_workspace_locator_resolution(
    workspace_root: &std::path::Path,
    route_result: &crate::RouteResult,
) -> Option<crate::post_route_policy::LocatorResolution> {
    if route_result.output_contract.locator_kind != crate::OutputLocatorKind::CurrentWorkspace {
        return None;
    }
    let locator_hint = route_result.output_contract.locator_hint.trim();
    if !locator_hint.is_empty()
        && !locator_hint_points_to_workspace_root(workspace_root, locator_hint)
    {
        return None;
    }
    Some(crate::post_route_policy::LocatorResolution::Direct(
        workspace_root.display().to_string(),
    ))
}

pub(super) fn locator_hint_names_workspace_root(
    workspace_root: &std::path::Path,
    locator_hint: &str,
) -> bool {
    let Some(root_name) = workspace_root.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    let normalized_root = normalize_locator_identity_token(root_name);
    let normalized_hint = normalize_locator_identity_token(locator_hint);
    !normalized_root.is_empty() && normalized_hint == normalized_root
}

pub(super) fn locator_hint_points_to_workspace_root(
    workspace_root: &std::path::Path,
    locator_hint: &str,
) -> bool {
    if locator_hint_names_workspace_root(workspace_root, locator_hint) {
        return true;
    }
    let locator_hint = locator_hint.trim();
    if locator_hint.is_empty() || locator_hint.contains('\n') {
        return false;
    }
    let candidate = std::path::Path::new(locator_hint);
    let candidate = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        workspace_root.join(candidate)
    };
    normalize_workspace_locator_path(&candidate) == normalize_workspace_locator_path(workspace_root)
}

pub(super) fn normalize_workspace_locator_path(path: &std::path::Path) -> std::path::PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

pub(super) fn normalize_locator_identity_token(value: &str) -> String {
    value
        .trim()
        .trim_matches(|c: char| {
            matches!(
                c,
                '"' | '\''
                    | '`'
                    | ','
                    | '.'
                    | '，'
                    | '。'
                    | ':'
                    | '：'
                    | ';'
                    | '；'
                    | ')'
                    | '('
                    | ']'
                    | '['
                    | '）'
                    | '（'
                    | '】'
                    | '【'
                    | '>'
                    | '<'
                    | '》'
                    | '《'
            )
        })
        .to_ascii_lowercase()
}

pub(super) fn should_attempt_auto_locator(route_result: &crate::RouteResult) -> bool {
    if route_result.needs_clarify && route_result.output_contract.locator_hint.trim().is_empty() {
        return false;
    }
    if route_result.output_contract.semantic_kind == crate::OutputSemanticKind::QuantityComparison
        && route_result.output_contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace
        && route_result.output_contract.locator_hint.trim().is_empty()
    {
        return false;
    }
    matches!(
        route_result.output_contract.locator_kind,
        crate::OutputLocatorKind::Path
            | crate::OutputLocatorKind::CurrentWorkspace
            | crate::OutputLocatorKind::Filename
    )
}

#[cfg(test)]
#[path = "ask_pipeline_locator_resolution_tests.rs"]
mod tests;

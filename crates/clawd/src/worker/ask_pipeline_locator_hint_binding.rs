use super::*;

pub(super) fn resolve_existing_workspace_locator_hint(
    state: &AppState,
    locator_hint: &str,
) -> Option<String> {
    let hint = locator_hint.trim();
    if hint.is_empty() || hint.contains('\n') {
        return None;
    }
    let path = std::path::Path::new(hint);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        state.skill_rt.workspace_root.join(path)
    };
    if !path.exists() {
        return None;
    }
    let canonical_path = path.canonicalize().unwrap_or(path);
    let canonical_root = state
        .skill_rt
        .workspace_root
        .canonicalize()
        .unwrap_or_else(|_| state.skill_rt.workspace_root.clone());
    canonical_path
        .starts_with(canonical_root)
        .then(|| canonical_path.display().to_string())
}

pub(super) fn locator_component_token(value: &str) -> Option<String> {
    let token = single_component_locator_hint(value)?;
    if token.len() < 2
        || token.chars().any(char::is_whitespace)
        || !token
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
    {
        return None;
    }
    Some(token)
}

use super::*;

pub(super) fn request_uses_filename_only_schema_token(req: &str) -> bool {
    let normalized = normalize_schema_token(req);
    [
        "filename_only",
        "file_name_only",
        "basename_only",
        "output_filename_only",
    ]
    .iter()
    .any(|token| normalized.contains(token))
}

pub(super) fn state_patch_deictic_reference_requires_clarify(state_patch: Option<&Value>) -> bool {
    state_patch_deictic_reference_target(state_patch).is_some_and(|target| {
        matches!(
            target,
            "unresolved_prior_object" | "missing_locator" | "ambiguous_locator"
        )
    })
}

pub(super) fn state_patch_deictic_reference_target(state_patch: Option<&Value>) -> Option<&str> {
    state_patch
        .and_then(|patch| patch.get("deictic_reference"))
        .and_then(Value::as_object)
        .and_then(|obj| obj.get("target"))
        .and_then(Value::as_str)
}

pub(super) fn state_patch_deictic_reference_is_resolved(state_patch: Option<&Value>) -> bool {
    state_patch_deictic_reference_target(state_patch).is_some_and(|target| {
        matches!(
            target,
            "current_action_result" | "current_turn_locator" | "comparison_result"
        )
    })
}

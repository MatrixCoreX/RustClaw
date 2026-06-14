use super::*;

pub(super) fn normalize_state_patch_text_token(text: &str) -> String {
    text.trim().to_ascii_lowercase().replace(['-', ' '], "_")
}

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

pub(super) fn state_patch_requests_filename_only_output(state_patch: Option<&Value>) -> bool {
    fn value_requests_filename_only(value: &Value) -> bool {
        match value {
            Value::String(text) => matches!(
                normalize_state_patch_text_token(text).as_str(),
                "filename_only" | "basename_only" | "file_name_only"
            ),
            Value::Array(items) => items.iter().any(value_requests_filename_only),
            Value::Object(map) => map.iter().any(|(key, value)| {
                matches!(
                    normalize_state_patch_text_token(key).as_str(),
                    "output_format"
                        | "output_shape"
                        | "format"
                        | "answer_format"
                        | "delivery_format"
                ) && value_requests_filename_only(value)
            }),
            _ => false,
        }
    }
    state_patch.is_some_and(value_requests_filename_only)
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

use serde_json::Value;

use super::{normalize_schema_token, IntentOutputContract};

fn state_patch_slice_mode(state_patch: Option<&Value>) -> Option<String> {
    let value = state_patch?.get("slice_mode")?.as_str()?.trim();
    let normalized = value.to_ascii_lowercase();
    matches!(normalized.as_str(), "head" | "tail" | "range").then_some(normalized)
}

fn state_patch_u64_field(state_patch: Option<&Value>, key: &str, max: u64) -> Option<u64> {
    let value = state_patch?.get(key)?;
    let parsed = match value {
        Value::Number(number) => number.as_u64(),
        Value::String(text) => text.trim().parse::<u64>().ok(),
        _ => None,
    }?;
    (parsed > 0).then_some(parsed.clamp(1, max))
}

pub(super) fn append_state_patch_slice_tokens_to_resolved_intent(
    mut resolved_user_intent: String,
    state_patch: Option<&Value>,
) -> String {
    let mut tokens = Vec::new();
    if let Some(mode) = state_patch_slice_mode(state_patch) {
        tokens.push(format!("slice_mode={mode}"));
    }
    if let Some(n) = state_patch_u64_field(state_patch, "slice_n", 500) {
        tokens.push(format!("slice_n={n}"));
    }
    if let Some(start) = state_patch_u64_field(state_patch, "slice_start", 1_000_000) {
        tokens.push(format!("slice_start={start}"));
    }
    if let Some(end) = state_patch_u64_field(state_patch, "slice_end", 1_000_000) {
        tokens.push(format!("slice_end={end}"));
    }
    if tokens.is_empty() {
        return resolved_user_intent;
    }
    if !resolved_user_intent.ends_with(char::is_whitespace) && !resolved_user_intent.is_empty() {
        resolved_user_intent.push(' ');
    }
    resolved_user_intent.push_str(&tokens.join(" "));
    resolved_user_intent
}

pub(super) fn append_state_patch_structured_field_selector_to_resolved_intent(
    mut resolved_user_intent: String,
    state_patch: Option<&Value>,
) -> String {
    let Some(selector) = structured_field_selector_from_state_patch(state_patch) else {
        return resolved_user_intent;
    };
    let token = format!("structured_field_selector={selector}");
    if resolved_user_intent
        .split_whitespace()
        .any(|part| part == token)
    {
        return resolved_user_intent;
    }
    if !resolved_user_intent.ends_with(char::is_whitespace) && !resolved_user_intent.is_empty() {
        resolved_user_intent.push(' ');
    }
    resolved_user_intent.push_str(&token);
    resolved_user_intent
}

pub(super) fn schema_key_is_structured_scalar_field_selector(key: &str) -> bool {
    matches!(
        key,
        "target_key"
            | "target_field"
            | "field_path"
            | "key_path"
            | "field_selector"
            | "structured_field_selector"
            | "json_pointer"
            | "json_path"
    )
}

pub(super) fn normalize_structured_field_selector(raw: Option<&str>) -> Option<String> {
    let selector = raw?.trim();
    if selector.is_empty()
        || selector.chars().count() > 256
        || selector.chars().any(char::is_control)
        || selector.chars().any(char::is_whitespace)
        || selector.contains('\\')
        || selector.contains("://")
        || selector.starts_with('{')
        || selector.starts_with('[')
        || selector.ends_with('}')
        || selector.ends_with(']')
    {
        return None;
    }
    if selector.starts_with('/') {
        return selector
            .split('/')
            .skip(1)
            .all(|segment| !segment.trim().is_empty())
            .then(|| selector.to_string());
    }
    let machine_selector = selector.chars().all(|ch| {
        ch.is_ascii_alphanumeric()
            || matches!(ch, '_' | '-' | '$' | '@' | '.' | '/' | '*' | '[' | ']')
    });
    machine_selector.then(|| selector.to_string())
}

fn structured_field_selector_from_state_patch_value(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::Object(map)) => {
            for (key, value) in map {
                let key = normalize_schema_token(key);
                if schema_key_is_structured_scalar_field_selector(&key) {
                    if let Some(selector) = value
                        .as_str()
                        .and_then(|text| normalize_structured_field_selector(Some(text)))
                    {
                        return Some(selector);
                    }
                }
                if let Some(selector) =
                    structured_field_selector_from_state_patch_value(Some(value))
                {
                    return Some(selector);
                }
            }
            None
        }
        Some(Value::Array(items)) => items
            .iter()
            .find_map(|value| structured_field_selector_from_state_patch_value(Some(value))),
        _ => None,
    }
}

fn structured_field_selector_from_state_patch(state_patch: Option<&Value>) -> Option<String> {
    structured_field_selector_from_state_patch_value(state_patch)
}

pub(super) fn apply_state_patch_structured_field_selector(
    output_contract: &mut IntentOutputContract,
    state_patch: Option<&Value>,
) -> Option<String> {
    let selector = structured_field_selector_from_state_patch(state_patch)?;
    if output_contract
        .self_extension
        .structured_field_selector
        .as_deref()
        .and_then(|value| normalize_structured_field_selector(Some(value)))
        .is_none()
    {
        output_contract.self_extension.structured_field_selector = Some(selector.clone());
    }
    Some(selector)
}

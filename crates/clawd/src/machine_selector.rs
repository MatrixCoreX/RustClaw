pub(crate) fn exact_machine_field_selector(selector: &str) -> Option<Vec<String>> {
    let fields = selector
        .split(|ch: char| ch.is_ascii_whitespace() || matches!(ch, ',' | ';'))
        .map(str::trim)
        .filter(|field| !field.is_empty())
        .collect::<Vec<_>>();
    if fields.is_empty()
        || fields.len() > 32
        || fields.iter().any(|field| {
            !valid_machine_key(field)
                || field.contains('*')
                || field.split('.').any(field_is_visible_text_boundary)
        })
    {
        return None;
    }
    let mut normalized = Vec::with_capacity(fields.len());
    for field in fields {
        if !normalized.iter().any(|existing| existing == field) {
            normalized.push(field.to_string());
        }
    }
    Some(normalized)
}

pub(crate) fn structured_json_satisfies_field_selector(selector: &str, output: &str) -> bool {
    let Some(fields) = exact_machine_field_selector(selector) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(output.trim()) else {
        return false;
    };
    fields.into_iter().all(|field| {
        let path = field.split('.').collect::<Vec<_>>();
        structured_json_contains_field_path(&value, &path)
    })
}

pub(crate) fn output_contract_requests_exact_scalar_field(
    route: &crate::IntentOutputContract,
    allowed_fields: &[&str],
) -> bool {
    output_contract_exact_scalar_field(route, allowed_fields).is_some()
}

pub(crate) fn output_contract_requests_exact_scalar_path(
    route: &crate::IntentOutputContract,
) -> bool {
    output_contract_requests_exact_scalar_field(route, &["path", "resolved_path"])
}

pub(crate) fn output_contract_requests_exact_list_path(
    route: &crate::IntentOutputContract,
) -> bool {
    if route.response_shape != crate::OutputResponseShape::Strict {
        return false;
    }
    let Some(fields) = route
        .selection
        .structured_field_selector
        .as_deref()
        .and_then(exact_machine_field_selector)
    else {
        return false;
    };
    matches!(fields.as_slice(), [field] if matches!(field.as_str(), "path" | "resolved_path"))
}

pub(crate) fn output_contract_exact_scalar_field(
    route: &crate::IntentOutputContract,
    allowed_fields: &[&str],
) -> Option<String> {
    if route.response_shape != crate::OutputResponseShape::Scalar {
        return None;
    }
    let Some(fields) = route
        .selection
        .structured_field_selector
        .as_deref()
        .and_then(exact_machine_field_selector)
    else {
        return None;
    };
    let [field] = fields.as_slice() else {
        return None;
    };
    allowed_fields
        .contains(&field.as_str())
        .then(|| field.to_string())
}

fn structured_json_contains_field_path(value: &serde_json::Value, path: &[&str]) -> bool {
    if path.is_empty() {
        return false;
    }
    match value {
        serde_json::Value::Object(object) => {
            if structured_object_contains_field_path(object, path) {
                return true;
            }
            object.iter().any(|(key, child)| {
                !field_is_visible_text_boundary(key)
                    && matches!(
                        child,
                        serde_json::Value::Object(_) | serde_json::Value::Array(_)
                    )
                    && structured_json_contains_field_path(child, path)
            })
        }
        serde_json::Value::Array(items) => items
            .iter()
            .any(|item| structured_json_contains_field_path(item, path)),
        _ => false,
    }
}

fn structured_object_contains_field_path(
    object: &serde_json::Map<String, serde_json::Value>,
    path: &[&str],
) -> bool {
    let Some(value) = object.get(path[0]) else {
        return false;
    };
    if path.len() == 1 {
        return true;
    }
    match value {
        serde_json::Value::Object(child) => {
            structured_object_contains_field_path(child, &path[1..])
        }
        serde_json::Value::Array(items) => items
            .iter()
            .any(|item| structured_json_contains_field_path(item, &path[1..])),
        _ => false,
    }
}

fn valid_machine_key(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
}

fn field_is_visible_text_boundary(key: &str) -> bool {
    matches!(key, "text" | "error_text")
}

#[cfg(test)]
#[path = "machine_selector_tests.rs"]
mod tests;

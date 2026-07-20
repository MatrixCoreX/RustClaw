use super::*;

pub(super) fn structured_field_display_line(
    _state: Option<&AppState>,
    field_path: &str,
    value: &serde_json::Value,
    value_text: Option<&str>,
    exists: bool,
    _prefer_english: bool,
) -> String {
    if !exists {
        return missing_extract_field_machine_answer(field_path);
    }
    let rendered = value_text
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToString::to_string)
        .or_else(|| value_scalar_text(value))
        .unwrap_or_else(|| {
            serde_json::to_string(value).unwrap_or_else(|_| "<unrenderable>".to_string())
        });
    format!("{field_path}: {rendered}")
}

pub(super) fn json_trimmed_str<'a>(value: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(|item| item.as_str())
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

pub(super) fn extract_field_has_non_exact_resolution(value: &serde_json::Value) -> bool {
    let Some(resolved) = json_trimmed_str(value, "resolved_field_path") else {
        return false;
    };
    let Some(requested) = json_trimmed_str(value, "field_path") else {
        return false;
    };
    if resolved.eq_ignore_ascii_case(requested) {
        return false;
    }
    !matches!(
        json_trimmed_str(value, "match_strategy"),
        Some("exact_path")
    )
}

pub(super) fn field_path_has_array_identity_selector(field_path: &str) -> bool {
    let field_path = field_path.trim();
    field_path.contains('[') && field_path.contains(']') && field_path.contains('=')
}

pub(super) fn extract_field_should_return_value_only(
    value: &serde_json::Value,
    field_path: &str,
) -> bool {
    field_path_has_array_identity_selector(field_path)
        || matches!(
            json_trimmed_str(value, "match_strategy"),
            Some("array_item_key_path")
        )
}

pub(super) fn extract_fields_direct_answer_candidate(
    state: Option<&AppState>,
    value: &serde_json::Value,
    response_shape: Option<crate::OutputResponseShape>,
    prefer_english: bool,
    allow_localized_template: bool,
) -> Option<String> {
    if !matches!(
        value.get("action").and_then(|v| v.as_str()),
        Some("extract_fields" | "read_fields")
    ) {
        return None;
    }
    let results = value.get("results")?.as_array()?;
    if results.is_empty() {
        return None;
    }
    if !allow_localized_template && results.iter().any(|item| field_result_is_missing(item)) {
        return None;
    }
    let lines = results
        .iter()
        .filter_map(|item| {
            let field_path = item
                .get("resolved_field_path")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .or_else(|| item.get("field_path")?.as_str().map(str::trim))?;
            if field_path.is_empty() {
                return None;
            }
            Some(structured_field_display_line(
                state,
                field_path,
                item.get("value").unwrap_or(&serde_json::Value::Null),
                item.get("value_text").and_then(|v| v.as_str()),
                item.get("exists")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                prefer_english,
            ))
        })
        .collect::<Vec<_>>();
    if lines.is_empty() {
        return None;
    }
    if matches!(
        response_shape,
        Some(crate::OutputResponseShape::OneSentence)
    ) {
        return Some(format!("{}.", lines.join("; ")));
    }
    Some(lines.join("\n"))
}

pub(super) fn field_result_is_missing(value: &serde_json::Value) -> bool {
    !value
        .get("exists")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

pub(super) fn extract_field_direct_answer_candidate(
    state: Option<&AppState>,
    value: &serde_json::Value,
    response_shape: Option<crate::OutputResponseShape>,
    prefer_english: bool,
    allow_localized_template: bool,
) -> Option<String> {
    if !matches!(
        value.get("action").and_then(|v| v.as_str()),
        Some("extract_field" | "read_field")
    ) {
        return None;
    }
    if matches!(response_shape, Some(crate::OutputResponseShape::Scalar)) {
        return None;
    }
    let field_path = value
        .get("resolved_field_path")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            value
                .get("field_path")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })?;
    let exists = value
        .get("exists")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !exists {
        if !allow_localized_template {
            return None;
        }
        return Some(missing_extract_field_machine_answer(field_path));
    }
    let field_value = value.get("value").unwrap_or(&serde_json::Value::Null);
    if matches!(
        field_value,
        serde_json::Value::Object(_) | serde_json::Value::Array(_)
    ) {
        if let Some(answer) =
            enum_field_direct_answer_candidate(state, field_path, field_value, prefer_english)
        {
            return Some(answer);
        }
        return None;
    }
    if extract_field_should_return_value_only(value, field_path) {
        return value_structured_text(
            field_value,
            value.get("value_text").and_then(|v| v.as_str()),
        );
    }
    Some(structured_field_display_line(
        state,
        field_path,
        field_value,
        value.get("value_text").and_then(|v| v.as_str()),
        exists,
        prefer_english,
    ))
}

pub(super) fn enum_field_direct_answer_candidate(
    _state: Option<&AppState>,
    field_path: &str,
    value: &serde_json::Value,
    _prefer_english: bool,
) -> Option<String> {
    let enum_value = match value {
        serde_json::Value::Object(map) => map.get("enum")?,
        serde_json::Value::Array(_) => value,
        _ => return None,
    };
    let values = enum_value.as_array()?;
    if values.is_empty() {
        return None;
    }
    let rendered_values = values
        .iter()
        .map(value_scalar_text)
        .collect::<Option<Vec<_>>>()?
        .into_iter()
        .filter(|item| !item.trim().is_empty())
        .map(|item| format!("`{item}`"))
        .collect::<Vec<_>>();
    if rendered_values.is_empty() {
        return None;
    }
    Some(enum_field_values_machine_answer(
        field_path,
        &rendered_values,
    ))
}

fn enum_field_values_machine_answer(field_path: &str, values: &[String]) -> String {
    let mut lines = vec![
        "message_key=clawd.msg.enum_field_values".to_string(),
        "reason_code=enum_field_values_observed".to_string(),
        "final_answer_shape=enum_field_values".to_string(),
    ];
    push_observed_machine_line(&mut lines, "field_path", field_path);
    lines.push(format!("value_count={}", values.len()));
    for (idx, value) in values.iter().enumerate() {
        push_observed_machine_line(&mut lines, &format!("value.{}", idx + 1), value);
    }
    lines.join("\n")
}

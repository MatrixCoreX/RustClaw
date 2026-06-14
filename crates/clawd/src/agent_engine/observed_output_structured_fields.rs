use super::*;

pub(super) fn structured_field_display_line(
    state: Option<&AppState>,
    field_path: &str,
    value: &serde_json::Value,
    value_text: Option<&str>,
    exists: bool,
    prefer_english: bool,
) -> String {
    if !exists {
        return observed_t_with_vars(
            state,
            "clawd.msg.structured_field_missing_display",
            "{field_path}: 不存在",
            "{field_path}: not found",
            prefer_english,
            &[("field_path", field_path)],
        );
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
        return Some(observed_t_with_vars(
            state,
            "clawd.msg.extract_field_missing",
            "未找到 {field_path} 字段",
            "field not found: {field_path}",
            prefer_english,
            &[("field_path", field_path)],
        ));
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
    state: Option<&AppState>,
    field_path: &str,
    value: &serde_json::Value,
    prefer_english: bool,
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
    let values_text = rendered_values.join(", ");
    Some(observed_t_with_vars(
        state,
        "clawd.msg.enum_field_values",
        "{field_path} 的枚举值是：{values}",
        "`{field_path}` enum values: {values}",
        prefer_english,
        &[("field_path", field_path), ("values", &values_text)],
    ))
}

pub(super) fn structured_keys_direct_answer_candidate(
    state: Option<&AppState>,
    value: &serde_json::Value,
    current_request: Option<&str>,
    response_shape: Option<crate::OutputResponseShape>,
    prefer_english: bool,
) -> Option<String> {
    if value.get("action").and_then(|v| v.as_str()) != Some("structured_keys") {
        return None;
    }
    let field_path = value
        .get("field_path")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or("");
    let exists = value
        .get("exists")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !exists {
        return Some(if field_path.is_empty() {
            observed_t(
                state,
                "clawd.msg.structured_keys_root_missing",
                "没有可列出的顶层键。",
                "No top-level keys are available to list.",
                prefer_english,
            )
        } else {
            observed_t_with_vars(
                state,
                "clawd.msg.structured_keys_field_missing",
                "{field_path} 字段不存在。",
                "Field `{field_path}` does not exist.",
                prefer_english,
                &[("field_path", field_path)],
            )
        });
    }
    let container_type = value
        .get("container_type")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    match container_type {
        "object" => {
            let keys = value
                .get("keys")
                .and_then(|v| v.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|v| v.as_str())
                        .map(str::trim)
                        .filter(|text| !text.is_empty())
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if keys.is_empty() {
                return None;
            }
            if let Some(target_key) = current_request
                .and_then(|request| structured_keys_presence_target_from_request(request, &keys))
            {
                let contains = keys
                    .iter()
                    .any(|key| key.eq_ignore_ascii_case(target_key.as_str()));
                return Some(structured_keys_presence_answer(
                    state,
                    &target_key,
                    contains,
                    prefer_english,
                ));
            }
            if matches!(
                response_shape,
                Some(crate::OutputResponseShape::OneSentence)
            ) {
                return None;
            }
            return Some(keys.join("\n"));
        }
        "array" => {
            let identity_values = value
                .get("identity_values")
                .and_then(|v| v.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|v| v.as_str())
                        .map(str::trim)
                        .filter(|text| !text.is_empty())
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if let Some(target_key) = current_request.and_then(|request| {
                structured_keys_presence_target_from_request(request, &identity_values)
            }) {
                let contains = identity_values
                    .iter()
                    .any(|key| key.eq_ignore_ascii_case(target_key.as_str()));
                return Some(structured_array_identity_presence_answer(
                    state,
                    &target_key,
                    contains,
                    prefer_english,
                ));
            }
            if !identity_values.is_empty()
                && !matches!(
                    response_shape,
                    Some(crate::OutputResponseShape::OneSentence)
                )
            {
                return Some(identity_values.join("\n"));
            }
            let indices = value
                .get("indices_preview")
                .and_then(|v| v.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.get("index").and_then(|v| v.as_u64()))
                        .map(|idx| idx.to_string())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if indices.is_empty() {
                return None;
            }
            if matches!(
                response_shape,
                Some(crate::OutputResponseShape::OneSentence)
            ) {
                return None;
            }
            return Some(indices.join("\n"));
        }
        _ => {}
    }
    Some(if field_path.is_empty() {
        observed_t(
            state,
            "clawd.msg.structured_keys_non_container_root",
            "这个位置不是对象或数组，没有可列出的键。",
            "This value is not an object or array, so there are no keys to list.",
            prefer_english,
        )
    } else {
        observed_t_with_vars(
            state,
            "clawd.msg.structured_keys_non_container_field",
            "{field_path} 不是对象或数组，没有可列出的键。",
            "`{field_path}` is not an object or array, so there are no keys to list.",
            prefer_english,
            &[("field_path", field_path)],
        )
    })
}

pub(super) fn structured_keys_presence_answer(
    state: Option<&AppState>,
    key: &str,
    contains: bool,
    prefer_english: bool,
) -> String {
    if contains {
        observed_t_with_vars(
            state,
            "clawd.msg.structured_keys_contains_key",
            "包含 {key} 字段",
            "Contains field `{key}`",
            prefer_english,
            &[("key", key)],
        )
    } else {
        observed_t_with_vars(
            state,
            "clawd.msg.structured_keys_missing_key",
            "不包含 {key} 字段",
            "Does not contain field `{key}`",
            prefer_english,
            &[("key", key)],
        )
    }
}

pub(super) fn structured_array_identity_presence_answer(
    state: Option<&AppState>,
    value: &str,
    contains: bool,
    prefer_english: bool,
) -> String {
    if contains {
        observed_t_with_vars(
            state,
            "clawd.msg.structured_array_identity_contains_value",
            "包含 {value}",
            "Contains `{value}`",
            prefer_english,
            &[("value", value)],
        )
    } else {
        observed_t_with_vars(
            state,
            "clawd.msg.structured_array_identity_missing_value",
            "不包含 {value}",
            "Does not contain `{value}`",
            prefer_english,
            &[("value", value)],
        )
    }
}

pub(super) fn structured_keys_presence_target_from_request(
    request: &str,
    keys: &[String],
) -> Option<String> {
    let tokens = structured_key_candidate_tokens(request);
    if tokens.is_empty() {
        return None;
    }
    let mut observed_mentions = Vec::new();
    for key in keys {
        if tokens.iter().any(|token| token.eq_ignore_ascii_case(key)) {
            push_unique_case_insensitive_string(&mut observed_mentions, key.clone());
        }
    }
    if observed_mentions.len() == 1 {
        return observed_mentions.into_iter().next();
    }
    let mut candidate_mentions = Vec::new();
    for token in explicit_structured_key_candidate_tokens(request) {
        if keys.iter().any(|key| key.eq_ignore_ascii_case(&token)) {
            continue;
        }
        push_unique_case_insensitive_string(&mut candidate_mentions, token);
    }
    for token in tokens {
        if !token_looks_like_structured_key_identifier(&token) {
            continue;
        }
        if keys.iter().any(|key| key.eq_ignore_ascii_case(&token)) {
            continue;
        }
        push_unique_case_insensitive_string(&mut candidate_mentions, token);
    }
    (candidate_mentions.len() == 1).then(|| candidate_mentions.remove(0))
}

pub(super) fn token_looks_like_structured_key_identifier(token: &str) -> bool {
    let token = token.trim();
    token.contains(['_', '.', '$'])
}

pub(super) fn explicit_structured_key_candidate_tokens(request: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let chars = request.char_indices().collect::<Vec<_>>();
    let mut idx = 0usize;
    while idx < chars.len() {
        let (start, ch) = chars[idx];
        if !matches!(ch, '`' | '\'' | '"') {
            idx += 1;
            continue;
        }
        let quote = ch;
        let content_start = start + ch.len_utf8();
        let mut end_idx = idx + 1;
        while end_idx < chars.len() {
            let (end, end_ch) = chars[end_idx];
            if end_ch == quote {
                let raw = request[content_start..end].trim();
                let token = raw.trim_matches(|ch: char| matches!(ch, '.' | '-' | '_' | '$'));
                if token.len() >= 2
                    && !token.contains(['/', '\\'])
                    && !token.chars().all(|ch| ch.is_ascii_digit())
                {
                    push_unique_case_insensitive_string(&mut tokens, token.to_string());
                }
                break;
            }
            end_idx += 1;
        }
        idx = end_idx.saturating_add(1);
    }
    tokens
}

pub(super) fn structured_key_candidate_tokens(request: &str) -> Vec<String> {
    let filename_candidates = crate::delivery_utils::extract_filename_candidates(request)
        .into_iter()
        .map(|candidate| candidate.to_ascii_lowercase())
        .collect::<Vec<_>>();
    let mut tokens = Vec::new();
    for raw in request.split(|ch: char| {
        !(ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '$' | '/' | '\\'))
    }) {
        let token = raw.trim_matches(|ch: char| matches!(ch, '.' | '-' | '_' | '$'));
        if token.len() < 2
            || token.contains(['/', '\\'])
            || token.chars().all(|ch| ch.is_ascii_digit())
            || crate::intent::locator_extractor::candidate_looks_like_dotted_version_number(token)
        {
            continue;
        }
        if filename_candidates
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(token))
        {
            continue;
        }
        push_unique_case_insensitive_string(&mut tokens, token.to_string());
    }
    tokens
}

pub(super) fn push_unique_case_insensitive_string(values: &mut Vec<String>, value: String) {
    if !values
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(&value))
    {
        values.push(value);
    }
}

pub(super) fn validate_structured_direct_answer_candidate(
    state: Option<&AppState>,
    value: &serde_json::Value,
    prefer_english: bool,
) -> Option<String> {
    if value.get("action").and_then(|v| v.as_str()) != Some("validate_structured") {
        return None;
    }
    let valid = value.get("valid")?.as_bool()?;
    let format = value
        .get("format")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("structured");
    if valid {
        return Some(observed_t_with_vars(
            state,
            "clawd.msg.validate_structured_pass",
            "通过：{format} 解析成功",
            "pass: {format} parsed successfully",
            prefer_english,
            &[("format", format)],
        ));
    }
    let reason = value
        .get("error_text")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("parse failed");
    Some(observed_t_with_vars(
        state,
        "clawd.msg.validate_structured_fail",
        "失败：{reason}",
        "fail: {reason}",
        prefer_english,
        &[("reason", reason)],
    ))
}

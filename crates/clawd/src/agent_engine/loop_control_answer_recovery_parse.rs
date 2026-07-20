use super::*;

pub(super) fn parse_structured_count_finding(output: &str) -> Option<StructuredCountFinding> {
    let value =
        crate::prompt_utils::parse_llm_json_raw_or_any_with_repair::<serde_json::Value>(output)?;
    let action = value
        .get("action")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())?
        .to_ascii_lowercase();
    if !matches!(action.as_str(), "count_inventory" | "inventory_dir") {
        return None;
    }
    let counts = value.get("counts")?;
    let total = counts.get("total").and_then(|value| value.as_u64())?;
    Some(StructuredCountFinding {
        path: value
            .get("path")
            .or_else(|| value.get("resolved_path"))
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string),
        total,
        files: counts.get("files").and_then(|value| value.as_u64()),
        dirs: counts.get("dirs").and_then(|value| value.as_u64()),
        hidden: counts.get("hidden").and_then(|value| value.as_u64()),
        recursive: value.get("recursive").and_then(|value| value.as_bool()),
    })
}

pub(super) fn parse_structured_search_finding(output: &str) -> Option<StructuredSearchFinding> {
    let value =
        crate::prompt_utils::parse_llm_json_raw_or_any_with_repair::<serde_json::Value>(output)?;
    let action = value
        .get("action")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())?
        .to_ascii_lowercase();
    if !structured_search_action_has_candidate_list(&action) {
        return None;
    }
    let raw_results = value.get("results").and_then(|value| value.as_array())?;
    let mut results = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for item in raw_results {
        let Some(token) = structured_search_result_token(item) else {
            continue;
        };
        if seen.insert(token.clone()) {
            results.push(token);
        }
    }
    if results.is_empty() {
        return None;
    }
    let count = value
        .get("count")
        .and_then(|value| value.as_u64())
        .map(|value| value as usize)
        .unwrap_or(results.len());
    Some(StructuredSearchFinding {
        action,
        count,
        results,
    })
}

pub(super) fn structured_search_action_has_candidate_list(action: &str) -> bool {
    matches!(
        action,
        "find_name" | "find_ext" | "find_entries" | "find_path" | "search"
    )
}

pub(super) fn structured_search_result_token(value: &serde_json::Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        return non_empty_structured_search_token(text);
    }
    let object = value.as_object()?;
    for key in [
        "path",
        "relative_path",
        "full_path",
        "name",
        "entry",
        "file",
        "filename",
    ] {
        if let Some(text) = object.get(key).and_then(|value| value.as_str()) {
            if let Some(token) = non_empty_structured_search_token(text) {
                return Some(token);
            }
        }
    }
    None
}

pub(super) fn non_empty_structured_search_token(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

pub(super) fn parse_log_analyze_finding(output: &str) -> Option<LogAnalyzeFinding> {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(output) {
        let path = value
            .get("path")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())?
            .to_string();
        let mut keyword_counts = value
            .get("keyword_counts")
            .and_then(|value| value.as_object())
            .map(|counts| {
                counts
                    .iter()
                    .filter_map(|(key, value)| value.as_u64().map(|count| (key.clone(), count)))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        return build_log_analyze_finding(path, &mut keyword_counts);
    }
    let path = extract_json_string_field(output, "path")?;
    let mut keyword_counts = extract_keyword_counts(output);
    build_log_analyze_finding(path, &mut keyword_counts)
}

pub(super) fn build_log_analyze_finding(
    path: String,
    keyword_counts: &mut Vec<(String, u64)>,
) -> Option<LogAnalyzeFinding> {
    keyword_counts.retain(|(key, count)| !key.trim().is_empty() && *count > 0);
    if keyword_counts.is_empty() {
        return None;
    }
    keyword_counts.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    let total_hits = keyword_counts.iter().map(|(_, count)| *count).sum::<u64>();
    Some(LogAnalyzeFinding {
        path,
        keyword_counts: keyword_counts.clone(),
        total_hits,
    })
}

pub(super) fn extract_keyword_counts(output: &str) -> Vec<(String, u64)> {
    let Some(marker_pos) = output.find("\"keyword_counts\"") else {
        return Vec::new();
    };
    let after_marker = &output[marker_pos + "\"keyword_counts\"".len()..];
    let Some(colon_pos) = after_marker.find(':') else {
        return Vec::new();
    };
    let after_colon = &after_marker[colon_pos + 1..];
    let Some(open_rel) = after_colon.find('{') else {
        return Vec::new();
    };
    let object_start = colon_pos + 1 + open_rel;
    let Some(object_end) = find_matching_json_object_end(after_marker, object_start) else {
        return Vec::new();
    };
    let inner = &after_marker[object_start + 1..object_end];
    inner
        .split(',')
        .filter_map(|part| {
            let (key, value) = part.split_once(':')?;
            let key = key.trim().trim_matches('"').to_string();
            let count = value.trim().parse::<u64>().ok()?;
            Some((key, count))
        })
        .collect()
}

pub(super) fn find_matching_json_object_end(input: &str, open_pos: usize) -> Option<usize> {
    let bytes = input.as_bytes();
    if bytes.get(open_pos).copied() != Some(b'{') {
        return None;
    }
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (idx, ch) in input.char_indices().skip_while(|(idx, _)| *idx < open_pos) {
        if in_string {
            if escaped {
                escaped = false;
                continue;
            }
            match ch {
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(idx);
                }
            }
            _ => {}
        }
    }
    None
}

pub(super) fn extract_json_string_field(input: &str, field: &str) -> Option<String> {
    let marker = format!("\"{field}\"");
    let mut offset = 0usize;
    while let Some(rel_pos) = input[offset..].find(&marker) {
        let marker_end = offset + rel_pos + marker.len();
        let after_marker = input[marker_end..].trim_start();
        let Some(after_colon) = after_marker.strip_prefix(':') else {
            offset = marker_end;
            continue;
        };
        return parse_json_string(after_colon.trim_start());
    }
    None
}

pub(super) fn parse_json_string(input: &str) -> Option<String> {
    let mut chars = input.chars();
    if chars.next()? != '"' {
        return None;
    }
    let mut out = String::new();
    let mut escaped = false;
    for ch in chars {
        if escaped {
            out.push(match ch {
                '"' => '"',
                '\\' => '\\',
                '/' => '/',
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                other => other,
            });
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '"' => return Some(out),
            other => out.push(other),
        }
    }
    None
}

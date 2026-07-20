use super::*;

pub(crate) fn normalize_read_range_excerpt(excerpt: &str) -> Option<String> {
    let lines = excerpt
        .lines()
        .map(str::trim_end)
        .map(|line| {
            let content = line
                .split_once('|')
                .filter(|(prefix, _)| {
                    !prefix.is_empty() && prefix.chars().all(|c| c.is_ascii_digit())
                })
                .map(|(_, rest)| rest.trim_start().to_string())
                .unwrap_or_else(|| line.trim().to_string());
            crate::visible_text::sanitize_user_visible_text(&content)
        })
        .collect::<Vec<_>>();
    if lines.is_empty() || lines.iter().all(|line| line.is_empty()) {
        None
    } else {
        Some(lines.join("\n"))
    }
}

pub(super) fn normalize_read_range_excerpt_for_direct_answer(
    _state: Option<&AppState>,
    excerpt: &str,
    _prefer_english: bool,
    _preserve_blank_lines: bool,
) -> Option<String> {
    let lines = excerpt
        .lines()
        .map(str::trim_end)
        .map(|line| {
            let content = line
                .split_once('|')
                .filter(|(prefix, _)| {
                    !prefix.is_empty() && prefix.chars().all(|c| c.is_ascii_digit())
                })
                .map(|(_, rest)| rest.trim_start().to_string())
                .unwrap_or_else(|| line.trim().to_string());
            crate::visible_text::sanitize_user_visible_text(&content)
        })
        .collect::<Vec<_>>();
    if lines.is_empty() || lines.iter().all(|line| line.is_empty()) {
        return None;
    }
    Some(lines.join("\n"))
}

pub(super) fn read_range_preserve_blank_lines(value: &serde_json::Value) -> bool {
    value.get("start_line").and_then(|v| v.as_u64()).is_some()
        && value.get("end_line").and_then(|v| v.as_u64()).is_some()
}

pub(crate) fn tail_read_range_direct_answer_candidate(
    body: &str,
    prefer_english: bool,
) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    if value.get("action").and_then(|v| v.as_str()) != Some("read_range") {
        return None;
    }
    if value.get("mode").and_then(|v| v.as_str()) != Some("tail") {
        return None;
    }
    let requested_n = value.get("requested_n").and_then(|v| v.as_u64())?;
    if requested_n == 0 || requested_n > 50 {
        return None;
    }
    value
        .get("excerpt")
        .and_then(|v| v.as_str())
        .and_then(|excerpt| {
            normalize_read_range_excerpt_for_direct_answer(None, excerpt, prefer_english, false)
        })
}

pub(super) fn read_range_observed_candidate(value: &serde_json::Value) -> Option<String> {
    let excerpt = value
        .get("excerpt")
        .and_then(|v| v.as_str())
        .and_then(normalize_read_range_excerpt)?;
    let path = value
        .get("resolved_path")
        .or_else(|| value.get("path"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty());
    Some(match path {
        Some(path) => format!("read_range path={path}\n{excerpt}"),
        None => excerpt,
    })
}

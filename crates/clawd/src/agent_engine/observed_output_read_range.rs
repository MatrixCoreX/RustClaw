use super::*;

pub(super) fn first_meaningful_excerpt_sentence(text: &str) -> Option<String> {
    let mut short_fallback = None;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty()
            || line.starts_with('#')
            || line.starts_with('<')
            || line.starts_with("```")
            || line.starts_with('|')
            || line.starts_with('-')
        {
            continue;
        }
        if short_fallback.is_none() {
            short_fallback = Some(line.to_string());
        }
        if line.chars().count() < 48 {
            continue;
        }
        let sentence = line
            .split_inclusive(['.', '。', '！', '!', '？', '?'])
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(line);
        return Some(sentence.to_string());
    }
    short_fallback
}

pub(super) fn content_excerpt_summary_direct_answer_candidate(
    route: Option<&crate::RouteResult>,
    body: &str,
) -> Option<String> {
    if !route.is_some_and(|route| {
        super::output_route_policy::route_contract_marker_is(
            route,
            crate::OutputSemanticKind::ContentExcerptSummary,
        )
    }) {
        return None;
    }
    let text = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("text")
                .or_else(|| value.get("excerpt"))
                .and_then(|value| value.as_str())
                .map(ToString::to_string)
        })
        .unwrap_or_else(|| body.to_string());
    first_meaningful_excerpt_sentence(&text)
}

pub(super) fn doc_parse_text_from_body(body: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("text")
                .or_else(|| value.get("excerpt"))
                .or_else(|| value.get("content"))
                .and_then(|value| value.as_str())
                .map(ToString::to_string)
        })
}

pub(super) fn contract_hint_bool_from_request(
    request_text: Option<&str>,
    key: &str,
) -> Option<bool> {
    let value = crate::intent_router::contract_test_hint_value(request_text?, key)?;
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

pub(super) fn content_presence_query_from_request(request_text: Option<&str>) -> Option<String> {
    crate::intent_router::contract_test_hint_value(request_text?, "selector_query")
        .map(|value| value.replace(['\r', '\n'], " "))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty() && value.len() <= 160)
}

pub(super) fn content_presence_case_insensitive_from_request(request_text: Option<&str>) -> bool {
    contract_hint_bool_from_request(request_text, "selector_case_insensitive")
        .or_else(|| contract_hint_bool_from_request(request_text, "selector_ignore_case"))
        .unwrap_or(true)
}

pub(super) fn find_content_presence_match_line(
    text: &str,
    query: &str,
    case_insensitive: bool,
) -> Option<(u64, String)> {
    let needle = if case_insensitive {
        query.to_lowercase()
    } else {
        query.to_string()
    };
    for (idx, line) in text.lines().enumerate() {
        let haystack = if case_insensitive {
            line.to_lowercase()
        } else {
            line.to_string()
        };
        if haystack.contains(&needle) {
            return Some((idx as u64 + 1, line.trim().to_string()));
        }
    }
    None
}

pub(super) fn doc_parse_content_presence_direct_answer_candidate(
    _state: Option<&AppState>,
    route: &crate::RouteResult,
    body: &str,
    request_text: Option<&str>,
    path_hint: Option<&str>,
    _prefer_english: bool,
) -> Option<String> {
    if !super::output_route_policy::route_contract_marker_is(
        route,
        crate::OutputSemanticKind::ContentPresenceCheck,
    ) {
        return None;
    }
    let query = content_presence_query_from_request(request_text)?;
    let text = doc_parse_text_from_body(body)?;
    let case_insensitive = content_presence_case_insensitive_from_request(request_text);
    let path = path_hint
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| route.output_contract.locator_hint.trim());
    if let Some((line, matched_text)) =
        find_content_presence_match_line(&text, &query, case_insensitive)
    {
        let location = if path.is_empty() {
            line.to_string()
        } else {
            format!("{path}:{line}")
        };
        return Some(content_presence_machine_answer(
            &query,
            path,
            case_insensitive,
            Some((line, &location, &matched_text)),
        ));
    }
    Some(content_presence_machine_answer(
        &query,
        path,
        case_insensitive,
        None,
    ))
}

fn content_presence_machine_answer(
    query: &str,
    path: &str,
    case_insensitive: bool,
    match_info: Option<(u64, &str, &str)>,
) -> String {
    let mut lines = vec![
        "message_key=clawd.msg.content_presence.observed".to_string(),
        "reason_code=content_presence_observed".to_string(),
        format!("contains={}", match_info.is_some()),
        format!("case_insensitive={case_insensitive}"),
    ];
    push_content_presence_machine_line(&mut lines, "query", query);
    if !path.trim().is_empty() {
        push_content_presence_machine_line(&mut lines, "path", path);
    }
    if let Some((line, location, matched_text)) = match_info {
        lines.push(format!("line={line}"));
        push_content_presence_machine_line(&mut lines, "location", location);
        push_content_presence_machine_line(&mut lines, "matched_text", matched_text);
    }
    lines.join("\n")
}

fn push_content_presence_machine_line(lines: &mut Vec<String>, key: &str, value: &str) {
    let value = crate::truncate_for_agent_trace(
        &crate::visible_text::sanitize_user_visible_text(value).replace('\n', " "),
    );
    lines.push(format!("{key}={value}"));
}

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
    state: Option<&AppState>,
    excerpt: &str,
    prefer_english: bool,
    preserve_blank_lines: bool,
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
    if !preserve_blank_lines && lines.iter().any(|line| line.is_empty()) {
        let blank = observed_t(
            state,
            "clawd.msg.read_range_blank_line",
            "（空行）",
            "(blank line)",
            prefer_english,
        );
        return Some(
            lines
                .into_iter()
                .map(|line| if line.is_empty() { blank.clone() } else { line })
                .collect::<Vec<_>>()
                .join("\n"),
        );
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

pub(super) fn bounded_read_range_direct_answer_candidate(
    body: &str,
    prefer_english: bool,
) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    if value.get("action").and_then(|v| v.as_str()) != Some("read_range") {
        return None;
    }
    let mode = value.get("mode").and_then(|v| v.as_str()).unwrap_or("");
    if !matches!(mode, "head" | "tail" | "range") {
        return None;
    }
    let bounded_lines = value
        .get("requested_n")
        .and_then(|v| v.as_u64())
        .or_else(|| {
            let start = value.get("start_line")?.as_u64()?;
            let end = value.get("end_line")?.as_u64()?;
            (end >= start).then_some(end - start + 1)
        })?;
    if bounded_lines == 0 || bounded_lines > 100 {
        return None;
    }
    value
        .get("excerpt")
        .and_then(|v| v.as_str())
        .and_then(|excerpt| {
            normalize_read_range_excerpt_for_direct_answer(None, excerpt, prefer_english, false)
        })
}

pub(super) fn bounded_read_range_output_path_candidate(body: &str) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    if value.get("action").and_then(|v| v.as_str()) != Some("read_range") {
        return None;
    }
    let mode = value.get("mode").and_then(|v| v.as_str()).unwrap_or("");
    if !matches!(mode, "head" | "tail" | "range") {
        return None;
    }
    let bounded_lines = value
        .get("requested_n")
        .and_then(|v| v.as_u64())
        .or_else(|| {
            let start = value.get("start_line")?.as_u64()?;
            let end = value.get("end_line")?.as_u64()?;
            (end >= start).then_some(end - start + 1)
        })?;
    if bounded_lines == 0 || bounded_lines > 100 {
        return None;
    }
    read_range_output_path(body)
}

pub(super) fn read_range_output_path(body: &str) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    if value.get("action").and_then(|v| v.as_str()) != Some("read_range") {
        return None;
    }
    value
        .get("resolved_path")
        .or_else(|| value.get("path"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(ToString::to_string)
}

pub(super) fn observed_read_path_matches_target(observed_path: &str, target: &str) -> bool {
    let observed_path = observed_path.trim();
    let target = target.trim();
    if observed_path.is_empty() || target.is_empty() {
        return false;
    }
    if observed_path == target {
        return true;
    }
    let observed = Path::new(observed_path);
    let target = Path::new(target);
    observed
        .canonicalize()
        .ok()
        .zip(target.canonicalize().ok())
        .is_some_and(|(observed, target)| observed == target)
}

pub(super) fn latest_bounded_read_range_direct_answer(
    loop_state: &LoopState,
    prefer_english: bool,
) -> Option<String> {
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .find_map(|step| {
            if !step.is_ok() || !matches!(step.skill.as_str(), "system_basic" | "fs_basic") {
                return None;
            }
            let output = step.output.as_deref()?.trim();
            bounded_read_range_direct_answer_candidate(output, prefer_english)
        })
}

pub(super) fn latest_bounded_read_range_output_path(loop_state: &LoopState) -> Option<String> {
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .find_map(|step| {
            if !step.is_ok() || !matches!(step.skill.as_str(), "system_basic" | "fs_basic") {
                return None;
            }
            let output = step.output.as_deref()?.trim();
            bounded_read_range_output_path_candidate(output)
        })
}

pub(super) fn bounded_read_range_direct_answer_for_target(
    loop_state: &LoopState,
    prefer_english: bool,
    target: &str,
) -> Option<String> {
    loop_state.executed_step_results.iter().find_map(|step| {
        if !step.is_ok() || !matches!(step.skill.as_str(), "system_basic" | "fs_basic") {
            return None;
        }
        let output = step.output.as_deref()?.trim();
        let path = read_range_output_path(output)?;
        observed_read_path_matches_target(&path, target)
            .then(|| bounded_read_range_direct_answer_candidate(output, prefer_english))
            .flatten()
    })
}

pub(super) fn bounded_read_range_output_path_for_target(
    loop_state: &LoopState,
    target: &str,
) -> Option<String> {
    loop_state.executed_step_results.iter().find_map(|step| {
        if !step.is_ok() || !matches!(step.skill.as_str(), "system_basic" | "fs_basic") {
            return None;
        }
        let output = step.output.as_deref()?.trim();
        let path = bounded_read_range_output_path_candidate(output)?;
        observed_read_path_matches_target(&path, target).then_some(path)
    })
}

pub(super) fn preferred_bounded_read_range_direct_answer(
    loop_state: &LoopState,
    prefer_english: bool,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    agent_run_context
        .and_then(|ctx| ctx.auto_locator_path.as_deref())
        .and_then(|target| {
            bounded_read_range_direct_answer_for_target(loop_state, prefer_english, target)
        })
        .or_else(|| latest_bounded_read_range_direct_answer(loop_state, prefer_english))
}

pub(super) fn preferred_bounded_read_range_output_path(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    agent_run_context
        .and_then(|ctx| ctx.auto_locator_path.as_deref())
        .and_then(|target| bounded_read_range_output_path_for_target(loop_state, target))
        .or_else(|| latest_bounded_read_range_output_path(loop_state))
}

pub(super) fn path_has_log_extension(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.trim().eq_ignore_ascii_case("log"))
}

pub(super) fn compact_delivery_match_text(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn answer_contains_observed_excerpt(answer: &str, excerpt: &str) -> bool {
    let answer = compact_delivery_match_text(answer);
    let excerpt = compact_delivery_match_text(excerpt);
    if excerpt.is_empty() {
        return true;
    }
    answer.contains(&excerpt) || excerpt.lines().all(|line| answer.contains(line))
}

pub(super) fn strip_observed_excerpt_prefix_from_answer(
    answer: &str,
    excerpt: &str,
) -> Option<String> {
    let answer = answer.trim();
    let excerpt = excerpt.trim();
    if answer.is_empty() || excerpt.is_empty() || !answer.starts_with(excerpt) {
        return None;
    }
    let stripped = answer[excerpt.len()..].trim();
    (!stripped.is_empty()).then(|| stripped.to_string())
}

pub(super) fn compose_content_excerpt_with_summary_answer(
    answer: &str,
    loop_state: &LoopState,
    prefer_english: bool,
    agent_run_context: Option<&AgentRunContext>,
) -> String {
    if !agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .is_some_and(|route| {
            super::output_route_policy::route_contract_marker_is(
                route,
                crate::OutputSemanticKind::ContentExcerptWithSummary,
            )
        })
    {
        return answer.trim().to_string();
    }
    let answer = answer.trim();
    let excerpt_path = preferred_bounded_read_range_output_path(loop_state, agent_run_context);
    let Some(excerpt) =
        preferred_bounded_read_range_direct_answer(loop_state, prefer_english, agent_run_context)
    else {
        return answer.to_string();
    };
    if excerpt_path.as_deref().is_some_and(path_has_log_extension) {
        return strip_observed_excerpt_prefix_from_answer(answer, &excerpt)
            .unwrap_or_else(|| answer.to_string());
    }
    if answer_contains_observed_excerpt(answer, &excerpt) {
        answer.to_string()
    } else if answer.is_empty() {
        excerpt
    } else {
        format!("{}\n\n{}", excerpt.trim(), answer)
    }
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

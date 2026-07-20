use super::*;
use std::path::Path;

pub(super) fn is_internal_missing_scalar_sentinel(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed == "<missing>" || trimmed.ends_with(": <missing>")
}

pub(super) fn scalar_count_diagnostic_machine_answer(diagnostic: &str) -> String {
    let diagnostic = crate::truncate_for_agent_trace(
        &crate::visible_text::sanitize_user_visible_text(diagnostic).replace('\n', " "),
    );
    [
        "message_key=clawd.msg.scalar_count.unreliable".to_string(),
        "reason_code=count_unreliable_diagnostic".to_string(),
        "final_answer_shape=scalar_count_unavailable".to_string(),
        format!("diagnostic={diagnostic}"),
    ]
    .join("\n")
}

pub(super) fn missing_extract_field_machine_answer(field_path: &str) -> String {
    let mut lines = vec![
        "message_key=clawd.msg.extract_field_missing".to_string(),
        "reason_code=extract_field_missing".to_string(),
        "final_answer_shape=missing_structured_field".to_string(),
        "exists=false".to_string(),
    ];
    push_observed_machine_line(&mut lines, "field_path", field_path);
    lines.join("\n")
}

pub(super) fn push_observed_machine_line(lines: &mut Vec<String>, key: &str, value: &str) {
    let value = crate::truncate_for_agent_trace(
        &crate::visible_text::sanitize_user_visible_text(value).replace('\n', " "),
    );
    lines.push(format!("{key}={value}"));
}

pub(super) fn direct_free_text_conflicts_with_request_language(
    candidate: &str,
    request_language_hint: &str,
) -> bool {
    output_route_policy::observed_text_conflicts_with_language_hint(
        candidate,
        request_language_hint,
    )
}

fn observed_answer_is_structured_machine_payload(candidate: &str) -> bool {
    matches!(
        serde_json::from_str::<serde_json::Value>(candidate.trim()),
        Ok(serde_json::Value::Object(_) | serde_json::Value::Array(_))
    ) || multi_field_machine_record_is_language_neutral(candidate)
}

pub(crate) fn multi_field_machine_record_is_language_neutral(candidate: &str) -> bool {
    let lines = candidate
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if markdown_machine_field_report_is_language_neutral(&lines) {
        return true;
    }
    if lines.len() >= 2 {
        return lines.iter().all(|line| machine_record_field(line));
    }
    let Some(line) = lines.first() else {
        return false;
    };
    let fields = line.split(',').map(str::trim).collect::<Vec<_>>();
    fields.len() >= 2 && fields.iter().all(|field| machine_record_field(field))
}

fn markdown_machine_field_report_is_language_neutral(lines: &[&str]) -> bool {
    let mut field_count = 0usize;
    for line in lines {
        if markdown_code_record_heading(line) {
            continue;
        }
        if let Some(fields) = markdown_code_record_inline_fields(line) {
            field_count += fields;
            continue;
        }
        let field = line
            .strip_prefix("- ")
            .or_else(|| line.strip_prefix("* "))
            .unwrap_or(line);
        if machine_record_field(field) {
            field_count += 1;
            continue;
        }
        return false;
    }
    field_count >= 2
}

fn markdown_code_record_heading(line: &str) -> bool {
    let Some(code) = line
        .strip_prefix('`')
        .and_then(|line| line.strip_suffix('`'))
    else {
        return false;
    };
    !code.trim().is_empty()
        && code
            .chars()
            .all(|ch| ch.is_ascii() && !ch.is_ascii_control())
}

fn markdown_code_record_inline_fields(line: &str) -> Option<usize> {
    let rest = line.strip_prefix('`')?;
    let (_, fields) = rest.split_once("`:")?;
    let fields = fields
        .split(',')
        .map(str::trim)
        .filter(|field| !field.is_empty())
        .collect::<Vec<_>>();
    (!fields.is_empty() && fields.iter().all(|field| machine_record_field(field)))
        .then_some(fields.len())
}

fn machine_record_field(field: &str) -> bool {
    let separator = field.find('=').or_else(|| field.find(':'));
    let Some(separator) = separator else {
        return false;
    };
    let (key, value_with_separator) = field.split_at(separator);
    let value = value_with_separator.get(1..).unwrap_or_default().trim();
    !key.is_empty()
        && !value.is_empty()
        && key
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
        && machine_record_value(value)
}

fn machine_record_value(value: &str) -> bool {
    if let Some(code) = value
        .strip_prefix('`')
        .and_then(|value| value.strip_suffix('`'))
    {
        return !code.trim().is_empty()
            && code
                .chars()
                .all(|ch| ch.is_ascii() && !ch.is_ascii_control());
    }
    value
        .split(',')
        .map(str::trim)
        .all(|item| !item.is_empty() && item.chars().all(|ch| ch.is_ascii_graphic()))
}

pub(super) fn observed_answer_language_compatible(
    candidate: &str,
    request_language_hint: &str,
) -> bool {
    observed_answer_is_structured_machine_payload(candidate)
        || !direct_free_text_conflicts_with_request_language(candidate, request_language_hint)
}

pub(super) fn observed_answer_language_compatible_for_route(
    route: Option<&crate::IntentOutputContract>,
    loop_state: &LoopState,
    auto_locator_path: Option<&str>,
    candidate: &str,
    request_language_hint: &str,
) -> bool {
    if let Some(route) = route {
        if route_requires_evidence_policy_grounded_direct_candidate(route)
            && evidence_policy_direct_candidate_satisfies_contract(
                route,
                loop_state,
                auto_locator_path,
                candidate,
            )
        {
            return true;
        }
    }
    observed_answer_language_compatible(candidate, request_language_hint)
}

fn read_range_candidate_looks_structured_artifact(candidate: &str) -> bool {
    let mut total = 0usize;
    let mut structural = 0usize;
    for line in candidate
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        total += 1;
        let starts_structural =
            line.starts_with(['{', '}', '[', ']', '<']) || line.ends_with(['{', '}', '[', ']']);
        let assignment_like = line
            .split_once('=')
            .is_some_and(|(left, right)| !left.trim().is_empty() && !right.trim().is_empty());
        let json_pair_like = line.trim_start().starts_with('"')
            && line
                .split_once(':')
                .is_some_and(|(left, right)| !left.trim().is_empty() && !right.trim().is_empty());
        if starts_structural || assignment_like || json_pair_like {
            structural += 1;
        }
    }
    total > 0 && structural.saturating_mul(2) >= total
}

pub(super) fn read_range_direct_candidate_conflicts_with_request_language(
    candidate: &str,
    request_language_hint: &str,
) -> bool {
    !read_range_candidate_looks_structured_artifact(candidate)
        && direct_free_text_conflicts_with_request_language(candidate, request_language_hint)
}

fn compare_paths_observed_candidate(body: &str) -> Option<String> {
    let raw_value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    let value = structured_observed_body_value(&raw_value);
    if value.get("action").and_then(|v| v.as_str()) != Some("compare_paths") {
        return None;
    }
    let left = value.get("left")?;
    let right = value.get("right")?;
    let left_path = left
        .get("resolved_path")
        .or_else(|| left.get("path"))
        .and_then(|v| v.as_str())
        .unwrap_or("left");
    let right_path = right
        .get("resolved_path")
        .or_else(|| right.get("path"))
        .and_then(|v| v.as_str())
        .unwrap_or("right");
    let left_size = left
        .get("size_bytes")
        .and_then(|v| v.as_u64())
        .map(|size| size.to_string())
        .unwrap_or_else(|| "-".to_string());
    let right_size = right
        .get("size_bytes")
        .and_then(|v| v.as_u64())
        .map(|size| size.to_string())
        .unwrap_or_else(|| "-".to_string());
    let comparison = value.get("comparison").and_then(|v| v.as_object());
    let same_path = comparison
        .and_then(|item| item.get("same_path"))
        .and_then(|v| v.as_bool())
        .map(|v| v.to_string())
        .unwrap_or_else(|| "-".to_string());
    let left_exists = left
        .get("exists")
        .and_then(|v| v.as_bool())
        .map(|v| v.to_string())
        .unwrap_or_else(|| "-".to_string());
    let right_exists = right
        .get("exists")
        .and_then(|v| v.as_bool())
        .map(|v| v.to_string())
        .unwrap_or_else(|| "-".to_string());
    let same_size = comparison
        .and_then(|item| item.get("same_size"))
        .and_then(|v| v.as_bool())
        .map(|v| v.to_string())
        .unwrap_or_else(|| "-".to_string());
    let size_delta = comparison
        .and_then(|item| item.get("size_delta_bytes"))
        .and_then(|v| v.as_i64())
        .map(|v| v.to_string())
        .unwrap_or_else(|| "-".to_string());
    Some(format!(
        "compare_paths left={left_path} left.exists={left_exists} left_size_bytes={left_size} right={right_path} right.exists={right_exists} right_size_bytes={right_size} same_path={same_path} same_size={same_size} size_delta_bytes={size_delta}"
    ))
}

fn observed_path_label(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(path)
        .to_string()
}

fn path_batch_facts_observed_candidate(value: &serde_json::Value) -> Option<String> {
    if value.get("action").and_then(|v| v.as_str()) != Some("path_batch_facts") {
        return None;
    }
    let facts = value.get("facts")?.as_array()?;
    if facts.is_empty() {
        return None;
    }
    let lines = facts
        .iter()
        .filter_map(|entry| {
            let entry = entry.as_object()?;
            let exists = entry
                .get("exists")
                .and_then(|value| value.as_bool())
                .unwrap_or(false);
            let fact = entry.get("fact").and_then(|value| value.as_object());
            let path = path_batch_fact_preferred_path(entry).unwrap_or("-");
            let label = observed_path_label(path);
            let kind = fact
                .and_then(|item| item.get("kind"))
                .and_then(|value| value.as_str())
                .unwrap_or("-");
            let size = fact
                .and_then(|item| item.get("size_bytes"))
                .and_then(|value| value.as_u64())
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_string());
            let modified = fact
                .and_then(|item| item.get("modified_ts"))
                .and_then(|value| value.as_i64())
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_string());
            Some(format!(
                "path_fact name={label} path={path} exists={exists} kind={kind} size_bytes={size} modified_ts={modified}"
            ))
        })
        .collect::<Vec<_>>();
    (!lines.is_empty()).then(|| format!("path_batch_facts\n{}", lines.join("\n")))
}

fn compact_log_analyze_excerpt(value: &serde_json::Value) -> Option<String> {
    let path = value
        .get("path")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())?;
    let keyword_counts = value
        .get("keyword_counts")
        .and_then(|v| v.as_object())
        .map(|map| {
            let mut pairs = map
                .iter()
                .filter_map(|(key, count)| count.as_u64().map(|count| (key.as_str(), count)))
                .collect::<Vec<_>>();
            pairs.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
            pairs
                .into_iter()
                .map(|(key, count)| format!("{key}={count}"))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let recent_matches = value
        .get("recent_matches")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str())
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .take(8)
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let level_counts = value
        .get("level_counts")
        .and_then(|v| v.as_object())
        .map(|map| {
            let mut pairs = map
                .iter()
                .filter_map(|(key, count)| count.as_u64().map(|count| (key.as_str(), count)))
                .collect::<Vec<_>>();
            pairs.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
            pairs
                .into_iter()
                .map(|(key, count)| format!("{key}={count}"))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let recent_notable_lines = value
        .get("recent_notable_lines")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str())
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .take(8)
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let tail_lines = value
        .get("tail_lines")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str())
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .take(12)
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let total_lines = value
        .get("total_lines")
        .and_then(|v| v.as_u64())
        .unwrap_or_default();

    let mut sections = vec![format!("log_analyze path={path} total_lines={total_lines}")];
    if !level_counts.is_empty() {
        sections.push(format!("level_counts: {}", level_counts.join(", ")));
    }
    if !keyword_counts.is_empty() {
        sections.push(format!("keyword_counts: {}", keyword_counts.join(", ")));
    }
    if !recent_notable_lines.is_empty() {
        sections.push(format!(
            "recent_notable_lines:\n- {}",
            recent_notable_lines.join("\n- ")
        ));
    }
    if !recent_matches.is_empty() {
        sections.push(format!(
            "recent_matches:\n- {}",
            recent_matches.join("\n- ")
        ));
    }
    if !tail_lines.is_empty() {
        sections.push(format!("tail_lines:\n- {}", tail_lines.join("\n- ")));
    }
    Some(sections.join("\n"))
}

fn count_inventory_observed_candidate(value: &serde_json::Value) -> Option<String> {
    if value.get("action").and_then(|v| v.as_str()) != Some("count_inventory") {
        return None;
    }
    let counts = value.get("counts")?;
    let mut lines = vec!["action=count_inventory".to_string()];
    for key in ["path", "resolved_path", "kind_filter"] {
        if let Some(text) = value
            .get(key)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            lines.push(format!("{key}={text}"));
        }
    }
    for key in ["files", "dirs", "total", "hidden", "total_size_bytes"] {
        if let Some(text) = counts.get(key).and_then(value_scalar_text) {
            lines.push(format!("count_{key}={text}"));
        }
    }
    if let Some(by_extension) = counts.get("by_extension").and_then(|v| v.as_object()) {
        let mut entries = by_extension
            .iter()
            .filter_map(|(ext, count)| {
                value_scalar_text(count).map(|count| format!("{ext}:{count}"))
            })
            .collect::<Vec<_>>();
        entries.sort();
        if !entries.is_empty() {
            lines.push(format!("count_by_extension={}", entries.join(", ")));
        }
    }
    (lines.len() > 1).then(|| lines.join("\n"))
}

fn count_inventory_observation_row(
    step: &crate::executor::StepExecutionResult,
) -> Option<(String, u64)> {
    if !step.is_ok() || !matches!(step.skill.as_str(), "system_basic" | "fs_basic") {
        return None;
    }
    let body = step.output.as_deref()?;
    let body = normalized_success_body_for_observed_output(body);
    let value = serde_json::from_str::<serde_json::Value>(body.trim()).ok()?;
    if value.get("action").and_then(|v| v.as_str()) != Some("count_inventory") {
        return None;
    }
    let total = value
        .get("counts")
        .and_then(|counts| counts.get("total"))
        .and_then(|v| v.as_u64())?;
    let path = value
        .get("resolved_path")
        .or_else(|| value.get("path"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or(step.step_id.as_str())
        .to_string();
    Some((path, total))
}

pub(super) fn multi_count_observation_guard_entry(loop_state: &LoopState) -> Option<String> {
    let rows = loop_state
        .executed_step_results
        .iter()
        .filter_map(count_inventory_observation_row)
        .collect::<Vec<_>>();
    if rows.len() < 2 {
        return None;
    }
    let mut lines = vec![
        "### MULTI_COUNT_OBSERVATIONS".to_string(),
        "delivery_constraint=cover_all_observed_count_rows".to_string(),
        format!("observed_count_rows={}", rows.len()),
    ];
    for (idx, (path, total)) in rows.iter().enumerate() {
        let row_no = idx + 1;
        lines.push(format!("observed_count.{row_no}.path={path}"));
        lines.push(format!("observed_count.{row_no}.count_total={total}"));
    }
    Some(lines.join("\n"))
}

fn validate_structured_observed_candidate(value: &serde_json::Value) -> Option<String> {
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
    Some(format!("validate_structured format={format} valid={valid}"))
}

pub(super) fn structured_observed_body(skill: &str, body: &str) -> Option<String> {
    if skill == "process_basic" {
        return process_basic_observed_candidate(body);
    }
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    let value = structured_observed_body_value(&value);
    match skill {
        "system_basic" => {
            let action = value.get("action").and_then(|v| v.as_str())?;
            match action {
                "read_range" => read_range_observed_candidate(&value),
                "inventory_dir" => inventory_dir_observed_candidate(&value),
                "count_inventory" => count_inventory_observed_candidate(&value),
                "tree_summary" => tree_summary_direct_answer_candidate(None, &value, true),
                "dir_compare" => dir_compare_direct_answer_candidate(None, &value, true),
                "validate_structured" => validate_structured_observed_candidate(&value),
                "compare_paths" => compare_paths_observed_candidate(body),
                "path_batch_facts" => path_batch_facts_observed_candidate(&value),
                _ => None,
            }
        }
        "config_basic" => validate_structured_observed_candidate(&value),
        "service_control" => service_control_summary_candidate(&value),
        "fs_search" | "fs_basic" => {
            if skill == "fs_basic" {
                match value.get("action").and_then(|v| v.as_str()) {
                    Some("inventory_dir") => return inventory_dir_observed_candidate(&value),
                    Some("read_range") => return read_range_observed_candidate(&value),
                    Some("count_inventory") => return count_inventory_observed_candidate(&value),
                    Some("path_batch_facts") => return path_batch_facts_observed_candidate(&value),
                    _ => {}
                }
            }
            fs_search_find_name_observed_candidate(&value)
                .or_else(|| fs_search_grep_text_observed_candidate(&value))
                .or_else(|| {
                    fs_search_direct_answer_candidate(None, &value, None, false, true, false)
                })
        }
        "log_analyze" => compact_log_analyze_excerpt(&value),
        _ => None,
    }
}

pub(super) fn structured_observed_body_value(value: &serde_json::Value) -> &serde_json::Value {
    value
        .get("extra")
        .filter(|extra| {
            extra.is_object()
                && extra
                    .get("action")
                    .and_then(serde_json::Value::as_str)
                    .is_some()
        })
        .unwrap_or(value)
}

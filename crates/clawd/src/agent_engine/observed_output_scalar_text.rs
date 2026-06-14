use super::*;

pub(super) fn trim_for_observed_prompt(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let mut out = trimmed.chars().take(max_chars).collect::<String>();
    out.push_str("\n...[truncated]");
    out
}

fn looks_like_structured_machine_output_line(line: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(line)
        .map(|value| value.is_object() || value.is_array())
        .unwrap_or(false)
}

pub(super) fn normalized_scalar_candidate(body: &str) -> Option<String> {
    let lines = body
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| *line != "exit=0")
        .filter(|line| !is_ignorable_shell_warning(line))
        .filter(|line| !looks_like_structured_machine_output_line(line))
        .collect::<Vec<_>>();
    (lines.len() == 1).then(|| lines[0].to_string())
}

fn numeric_scalar_text(text: &str) -> bool {
    let trimmed = text.trim();
    !trimmed.is_empty() && trimmed.parse::<f64>().is_ok()
}

pub(super) fn scalar_count_diagnostic_line_for_answer(
    answer: &str,
    route: Option<&crate::RouteResult>,
    loop_state: &LoopState,
) -> Option<String> {
    let route = route?;
    if !route_requests_scalar_count(route) || !numeric_scalar_text(answer) {
        return None;
    }
    let observed = extract_latest_generic_successful_output(loop_state)?;
    if observed.skill == "archive_basic" && archive_list_summary_from_body(&observed.body).is_some()
    {
        return None;
    }
    let lines = observed
        .body
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| *line != "exit=0")
        .filter(|line| !is_ignorable_shell_warning(line))
        .filter(|line| !looks_like_structured_machine_output_line(line))
        .collect::<Vec<_>>();
    if lines.len() <= 1 {
        return None;
    }
    lines
        .into_iter()
        .find(|line| !numeric_scalar_text(line))
        .map(ToString::to_string)
}

pub(super) fn value_scalar_text(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Null => Some("null".to_string()),
        serde_json::Value::Bool(v) => Some(v.to_string()),
        serde_json::Value::Number(v) => Some(v.to_string()),
        serde_json::Value::String(v) => Some(v.trim().to_string()).filter(|v| !v.is_empty()),
        _ => None,
    }
}

pub(super) fn value_structured_text(
    value: &serde_json::Value,
    value_text: Option<&str>,
) -> Option<String> {
    value_text
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToString::to_string)
        .or_else(|| serde_json::to_string(value).ok())
}

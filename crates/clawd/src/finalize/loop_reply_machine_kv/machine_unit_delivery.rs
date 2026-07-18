use serde_json::Value;

use crate::agent_engine::{AgentRunContext, LoopState};

pub(super) fn current_delivery_has_conflicting_values_for_requested_keys(
    current: &str,
    requested_summary: &str,
) -> bool {
    requested_machine_keys(requested_summary)
        .into_iter()
        .any(|key| machine_kv_values_for_key(current, &key).len() > 1)
}

pub(super) fn current_delivery_contains_all_requested_machine_units(
    current: &str,
    requested_summary: &str,
) -> bool {
    if current_delivery_is_machine_kv_only(current) {
        return false;
    }
    let requested_units = machine_kv_units(requested_summary);
    if requested_units.is_empty() {
        return false;
    }
    let current_units = machine_kv_units(current);
    requested_units.iter().all(|unit| {
        current_units.iter().any(|current| current == unit)
            || requested_machine_unit_matches_labeled_line(current, unit)
    })
}

fn requested_machine_unit_matches_labeled_line(current: &str, requested_unit: &str) -> bool {
    let Some((requested_key, requested_value)) = requested_unit.split_once('=') else {
        return false;
    };
    current
        .lines()
        .filter_map(|line| labeled_machine_field_value(line, requested_key))
        .any(|value| value == requested_value)
}

fn labeled_machine_field_value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let line = line
        .trim()
        .strip_prefix("- ")
        .or_else(|| line.trim().strip_prefix("* "))
        .or_else(|| line.trim().strip_prefix("+ "))
        .unwrap_or_else(|| line.trim());
    let rest = line.strip_prefix(key)?;
    let value = rest
        .strip_prefix('=')
        .or_else(|| rest.strip_prefix(':'))?
        .trim();
    (!value.is_empty()).then_some(value)
}

pub(super) fn latest_publishable_delivery_with_requested_machine_units(
    loop_state: &LoopState,
    delivery_messages: &[String],
    requested_summary: &str,
) -> Option<String> {
    if machine_kv_units(requested_summary).is_empty() {
        return None;
    }
    for step in loop_state.executed_step_results.iter().rev() {
        if !step.is_ok() || !matches!(step.skill.as_str(), "respond" | "synthesize_answer") {
            continue;
        }
        if let Some(candidate) = step.output.as_deref().and_then(|candidate| {
            publishable_rich_delivery_with_requested_machine_units(candidate, requested_summary)
        }) {
            return Some(candidate);
        }
    }
    for candidate in [
        loop_state.last_user_visible_respond.as_deref(),
        loop_state.last_publishable_synthesis_output.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        if let Some(candidate) =
            publishable_rich_delivery_with_requested_machine_units(candidate, requested_summary)
        {
            return Some(candidate);
        }
    }
    for candidate in loop_state
        .delivery_messages
        .iter()
        .rev()
        .chain(delivery_messages.iter().rev())
    {
        if let Some(candidate) =
            publishable_rich_delivery_with_requested_machine_units(candidate, requested_summary)
        {
            return Some(candidate);
        }
    }
    None
}

fn publishable_rich_delivery_with_requested_machine_units(
    candidate: &str,
    requested_summary: &str,
) -> Option<String> {
    let candidate = candidate.trim();
    if candidate.is_empty()
        || current_delivery_is_machine_kv_only(candidate)
        || crate::finalize::parse_delivery_token(candidate).is_some()
        || crate::finalize::looks_like_planner_artifact(candidate)
        || crate::finalize::looks_like_internal_trace_artifact(candidate)
        || crate::finalize::is_execution_summary_message(candidate)
        || !current_delivery_contains_all_requested_machine_units(candidate, requested_summary)
    {
        return None;
    }
    Some(candidate.to_string())
}

pub(super) fn patch_current_delivery_empty_requested_machine_fields(
    current: &str,
    requested_summary: &str,
) -> Option<String> {
    let pairs = requested_machine_summary_pairs(requested_summary);
    if pairs.is_empty() || current.trim().is_empty() {
        return None;
    }
    let mut changed = false;
    let patched = current
        .lines()
        .map(|line| {
            if let Some(patched) = patch_empty_machine_field_line(line, &pairs) {
                changed = true;
                patched
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    changed.then_some(patched)
}

pub(super) fn patch_current_delivery_conflicting_requested_machine_fields(
    current: &str,
    requested_summary: &str,
) -> Option<String> {
    if current_delivery_is_machine_kv_only(current) {
        return None;
    }
    let pairs = requested_machine_summary_pairs(requested_summary);
    if pairs.is_empty() || current.trim().is_empty() {
        return None;
    }
    let current_values = pairs
        .iter()
        .map(|(key, _)| {
            current
                .lines()
                .filter_map(|line| labeled_machine_field_value(line, key))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    if current_values.iter().any(|values| values.len() != 1) {
        return None;
    }

    let mut changed = false;
    let patched = current
        .lines()
        .map(|line| {
            for (key, expected) in &pairs {
                let Some(actual) = labeled_machine_field_value(line, key) else {
                    continue;
                };
                if actual == expected {
                    break;
                }
                let key_offset = line.find(key)?;
                let delimiter_offset = key_offset + key.len();
                let delimiter = line.as_bytes().get(delimiter_offset).copied()?;
                if !matches!(delimiter, b'=' | b':') {
                    break;
                }
                changed = true;
                return Some(format!(
                    "{}{} {}",
                    &line[..delimiter_offset],
                    delimiter as char,
                    expected
                ));
            }
            Some(line.to_string())
        })
        .collect::<Option<Vec<_>>>()?
        .join("\n");
    changed.then_some(patched)
}

pub(super) fn requested_machine_summary_pairs(requested_summary: &str) -> Vec<(String, String)> {
    machine_kv_units(requested_summary)
        .into_iter()
        .filter_map(|unit| {
            let (key, value) = unit.split_once('=')?;
            Some((key.to_string(), value.to_string()))
        })
        .collect()
}

fn patch_empty_machine_field_line(line: &str, pairs: &[(String, String)]) -> Option<String> {
    let trimmed = line.trim();
    for (key, value) in pairs {
        if empty_machine_field_line(trimmed, key) {
            let indent_len = line.len().saturating_sub(line.trim_start().len());
            let indent = &line[..indent_len];
            return Some(format!("{indent}{key}={value}"));
        }
    }
    None
}

fn empty_machine_field_line(line: &str, key: &str) -> bool {
    let Some(rest) = line.strip_prefix(key) else {
        return false;
    };
    matches!(
        rest.trim(),
        "" | "="
            | ":"
            | "=null"
            | ":null"
            | "= null"
            | ": null"
            | "=none"
            | ":none"
            | "= none"
            | ": none"
            | "=<none>"
            | ":<none>"
            | "= <none>"
            | ": <none>"
    )
}

fn requested_machine_keys(requested_summary: &str) -> Vec<String> {
    let mut keys = machine_kv_unit_keys(requested_summary);
    for marker in bare_machine_markers(requested_summary) {
        if !keys.iter().any(|key| key == &marker) {
            keys.push(marker);
        }
    }
    keys
}

fn machine_kv_values_for_key(text: &str, requested_key: &str) -> Vec<String> {
    let mut values = Vec::new();
    for unit in machine_kv_units(text) {
        let Some((key, value)) = unit.split_once('=') else {
            continue;
        };
        if key != requested_key || values.iter().any(|existing| existing == value) {
            continue;
        }
        values.push(value.to_string());
    }
    values
}

pub(super) fn strict_machine_field_contract_requested(
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    agent_run_context
        .and_then(|ctx| ctx.turn_analysis.as_ref())
        .and_then(|analysis| analysis.state_patch.as_ref())
        .is_some_and(state_patch_has_required_machine_field_contract)
}

fn state_patch_has_required_machine_field_contract(value: &Value) -> bool {
    match value {
        Value::Object(object) => object.iter().any(|(key, child)| {
            let key = normalized_state_patch_key(key);
            matches!(
                key.as_str(),
                "required_field" | "required_machine_field" | "required_machine_fields"
            ) || state_patch_has_required_machine_field_contract(child)
        }),
        Value::Array(items) => items
            .iter()
            .any(state_patch_has_required_machine_field_contract),
        Value::String(_) | Value::Number(_) | Value::Bool(_) | Value::Null => false,
    }
}

pub(super) fn normalized_state_patch_key(key: &str) -> String {
    key.trim()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
        .collect::<String>()
        .to_ascii_lowercase()
}

pub(super) fn current_delivery_is_publishable_evidence_summary(
    route: &crate::IntentOutputContract,
    current: &str,
    requested_summary: &str,
) -> bool {
    if matches!(route.response_shape, crate::OutputResponseShape::FileToken)
        || !route_allows_model_language_delivery(route)
        || (machine_kv_units(requested_summary).is_empty()
            && bare_machine_markers(requested_summary).is_empty())
    {
        return false;
    }
    let current = current.trim();
    if current.is_empty()
        || current.starts_with('{')
        || current.starts_with('[')
        || crate::finalize::parse_delivery_token(current).is_some()
        || crate::finalize::looks_like_planner_artifact(current)
        || crate::finalize::looks_like_internal_trace_artifact(current)
        || crate::finalize::is_execution_summary_message(current)
        || super::super::looks_like_raw_command_snapshot(current)
        || super::super::looks_like_structured_machine_output(current)
        || current_delivery_is_machine_kv_only(current)
    {
        return false;
    }
    let current_chars = current.chars().count();
    let summary_chars = requested_summary.trim().chars().count();
    let nonempty_lines = current
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .count();
    let token_count = current.split_whitespace().count();
    current_chars > summary_chars.saturating_add(16)
        && (nonempty_lines > 1 || token_count >= 6 || current_chars >= 48)
}

fn route_allows_model_language_delivery(route: &crate::IntentOutputContract) -> bool {
    crate::evidence_policy::final_answer_shape_for_output_contract(route)
        .is_some_and(|shape| shape.allows_model_language())
        || matches!(
            route.response_shape,
            crate::OutputResponseShape::Free | crate::OutputResponseShape::OneSentence
        )
}

pub(super) fn current_delivery_is_machine_kv_only(current: &str) -> bool {
    let mut saw_line = false;
    for line in current
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        saw_line = true;
        let units = machine_kv_units(line);
        if units.is_empty() {
            return false;
        }
        let unit_text = units.join(" ");
        if unit_text != line {
            return false;
        }
    }
    saw_line
}

pub(super) fn current_delivery_has_values_for_requested_marker_summary(
    current: &str,
    requested_summary: &str,
) -> bool {
    let requested_markers = bare_machine_markers(requested_summary);
    !requested_markers.is_empty()
        && requested_markers
            .iter()
            .all(|marker| current_delivery_has_value_for_marker(current, marker))
}

pub(super) fn bare_machine_markers(text: &str) -> Vec<String> {
    let tokens = text
        .split_whitespace()
        .map(|token| {
            token.trim_matches(|ch: char| {
                matches!(
                    ch,
                    ',' | '.' | ';' | ':' | '(' | ')' | '[' | ']' | '{' | '}'
                )
            })
        })
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    if tokens.is_empty() || tokens.iter().any(|token| token.contains('=')) {
        return Vec::new();
    }
    tokens
        .into_iter()
        .filter(|token| valid_machine_unit_key(token))
        .map(str::to_string)
        .collect()
}

fn current_delivery_has_value_for_marker(current: &str, marker: &str) -> bool {
    let marker = marker.trim();
    if marker.is_empty() {
        return false;
    }
    current.lines().any(|line| {
        let line = line.trim();
        if let Some(value) = line.strip_prefix(format!("{marker}=").as_str()) {
            return !value.trim().is_empty();
        }
        if let Some(value) = line.strip_prefix(format!("{marker}:").as_str()) {
            return !value.trim().is_empty();
        }
        false
    })
}

pub(super) fn route_required_machine_evidence_is_present_in_current_delivery(
    route: &crate::IntentOutputContract,
    current: &str,
) -> bool {
    if !route.requires_content_evidence {
        return false;
    }
    let current_keys = machine_kv_unit_keys(current);
    if current_keys.is_empty() {
        return false;
    }
    crate::evidence_policy::required_evidence_fields_for_output_contract(route)
        .iter()
        .any(|field| current_keys.iter().any(|key| key == field))
}

pub(super) fn machine_kv_units_strictly_extend(current: &str, requested_summary: &str) -> bool {
    let current_units = machine_kv_units(current);
    let requested_units = machine_kv_units(requested_summary);
    !requested_units.is_empty()
        && current_units.len() > requested_units.len()
        && requested_units
            .iter()
            .all(|unit| current_units.iter().any(|current| current == unit))
}

pub(super) fn machine_kv_units(text: &str) -> Vec<String> {
    text.split_whitespace()
        .filter_map(|token| {
            let token = token.trim_matches(|ch: char| {
                matches!(
                    ch,
                    ',' | '.' | ';' | ':' | '(' | ')' | '[' | ']' | '{' | '}'
                )
            });
            let (key, value) = token.split_once('=')?;
            if valid_machine_unit_key(key) && !value.trim().is_empty() {
                Some(format!("{}={}", key.trim(), value.trim()))
            } else {
                None
            }
        })
        .collect()
}

fn machine_kv_unit_keys(text: &str) -> Vec<String> {
    machine_kv_units(text)
        .into_iter()
        .filter_map(|unit| unit.split_once('=').map(|(key, _)| key.to_string()))
        .collect()
}

pub(super) fn valid_machine_unit_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
}

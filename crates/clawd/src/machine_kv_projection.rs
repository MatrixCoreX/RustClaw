pub(crate) fn requested_machine_kv_summary_from_observations(
    input: &str,
    observed_texts: &[String],
) -> Option<String> {
    let pairs = requested_machine_kv_pairs(input);
    let markers = requested_machine_markers(input);
    if (pairs.is_empty() && markers.is_empty()) || observed_texts.is_empty() {
        return None;
    }
    if !markers
        .iter()
        .all(|marker| machine_marker_is_observed(marker, observed_texts))
    {
        return None;
    }
    if !pairs.is_empty()
        && !pairs
            .iter()
            .any(|pair| machine_kv_pair_has_observed_value(pair, observed_texts))
    {
        return None;
    }
    if !machine_kv_pairs_grounded_by_observation(&pairs, observed_texts) {
        return None;
    }
    let mut parts = markers;
    parts.extend(pairs.iter().map(|(key, value)| format!("{key}={value}")));
    Some(parts.join(" "))
}

pub(crate) fn requested_machine_kv_summary_from_observation_inputs<'a>(
    inputs: impl IntoIterator<Item = &'a str>,
    observed_texts: &[String],
) -> Option<String> {
    inputs
        .into_iter()
        .find_map(|input| requested_machine_kv_summary_from_observations(input, observed_texts))
}

pub(crate) fn collect_machine_kv_surfaces_from_json(
    value: &serde_json::Value,
    surfaces: &mut Vec<String>,
) {
    match value {
        serde_json::Value::String(text) => {
            push_unique_machine_kv_surface(surfaces, text);
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_machine_kv_surfaces_from_json(item, surfaces);
            }
        }
        serde_json::Value::Object(object) => {
            for child in object.values() {
                collect_machine_kv_surfaces_from_json(child, surfaces);
            }
        }
        serde_json::Value::Number(_) | serde_json::Value::Bool(_) | serde_json::Value::Null => {}
    }
}

pub(crate) fn push_unique_machine_kv_surface(surfaces: &mut Vec<String>, value: &str) {
    let value = value.trim();
    if value.is_empty() {
        return;
    }
    if !surfaces.iter().any(|existing| existing == value) {
        surfaces.push(value.to_string());
    }
}

fn requested_machine_kv_pairs(input: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    let tokens = input.split_whitespace().collect::<Vec<_>>();
    let mut index = 0;
    while index < tokens.len() {
        let token = tokens[index];
        let token = trim_machine_kv_token(token);
        let Some((key, value)) = token.split_once('=') else {
            index += 1;
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        if valid_machine_key(key) && valid_machine_value_atom(value) {
            let mut segments = vec![value.to_string()];
            let mut next_index = index + 1;
            while next_index < tokens.len() {
                let next = trim_machine_kv_token(tokens[next_index]);
                if next.is_empty() || next.contains('=') || !valid_machine_value_continuation(next)
                {
                    break;
                }
                segments.push(next.to_string());
                next_index += 1;
            }
            let value = segments.join(" ");
            let pair = (key.to_string(), value);
            if !pairs.contains(&pair) {
                pairs.push(pair);
            }
            index = next_index;
        } else {
            index += 1;
        }
    }
    collect_embedded_machine_kv_pairs(input, &mut pairs);
    pairs
}

fn collect_embedded_machine_kv_pairs(input: &str, pairs: &mut Vec<(String, String)>) {
    for segment in input.split(machine_token_boundary) {
        let token = trim_machine_kv_token(segment);
        let Some((key, value)) = token.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        if valid_machine_key(key) && valid_machine_value_atom(value) {
            let pair = (key.to_string(), value.to_string());
            if !pairs.iter().any(|(existing_key, _)| existing_key == key) {
                pairs.push(pair);
            }
        }
    }
}

fn requested_machine_markers(input: &str) -> Vec<String> {
    let mut markers = Vec::new();
    for segment in input.split(machine_token_boundary) {
        let token = trim_machine_marker_token(segment);
        if valid_machine_marker(token) && !markers.iter().any(|existing| existing == token) {
            markers.push(token.to_string());
        }
    }
    markers
        .iter()
        .filter(|candidate| {
            !markers.iter().any(|other| {
                other.len() > candidate.len()
                    && other.starts_with(candidate.as_str())
                    && other.as_bytes().get(candidate.len()) == Some(&b'.')
            })
        })
        .cloned()
        .collect()
}

fn machine_token_boundary(ch: char) -> bool {
    !(ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/' | ':' | '=' | '@' | ','))
}

fn trim_machine_kv_token(token: &str) -> &str {
    token
        .trim_matches(|ch: char| {
            !(ch.is_ascii_alphanumeric()
                || matches!(ch, '_' | '-' | '.' | '/' | ':' | '=' | '@' | ','))
        })
        .trim_end_matches(|ch: char| matches!(ch, ',' | ';' | '.'))
}

fn trim_machine_marker_token(token: &str) -> &str {
    token
        .trim_matches(|ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.')))
        .trim_end_matches(|ch: char| matches!(ch, ',' | ';' | '.'))
}

fn valid_machine_key(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
}

fn valid_machine_marker(value: &str) -> bool {
    valid_machine_key(value)
        && value.contains('.')
        && value
            .split('.')
            .all(|part| !part.is_empty() && part.chars().any(|ch| ch.is_ascii_alphabetic()))
}

fn valid_machine_value_atom(value: &str) -> bool {
    if let Some((key, nested_value)) = value.split_once('=') {
        return !nested_value.contains('=')
            && valid_machine_key(key)
            && valid_machine_scalar_value_atom(nested_value);
    }
    valid_machine_scalar_value_atom(value)
}

fn valid_machine_scalar_value_atom(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 160
        && value.chars().all(|ch| {
            ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/' | ':' | '@' | ',')
        })
}

fn valid_machine_value_continuation(value: &str) -> bool {
    valid_machine_value_atom(value)
        && value
            .chars()
            .any(|ch| ch.is_ascii_digit() || matches!(ch, '_' | '-' | '.' | '/' | ':' | '@'))
}

pub(crate) fn observed_machine_text_fragments_from_journal(
    journal: &crate::task_journal::TaskJournal,
) -> Vec<String> {
    let mut values = Vec::new();
    for step in &journal.step_results {
        if step.status != crate::executor::StepExecutionStatus::Ok {
            continue;
        }
        let Some(output) = step.output_excerpt.as_deref() else {
            continue;
        };
        collect_machine_text_fragments_from_output(output, &mut values);
    }
    values.sort();
    values.dedup();
    values
}

pub(crate) fn collect_machine_text_fragments_from_output(output: &str, values: &mut Vec<String>) {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return;
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        collect_machine_text_fragments_from_json(&value, values);
    } else {
        values.push(trimmed.to_string());
    }
}

fn collect_machine_text_fragments_from_json(value: &serde_json::Value, values: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(obj) => {
            for key in ["excerpt", "content_excerpt", "text", "output"] {
                if let Some(text) = obj
                    .get(key)
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                {
                    values.push(read_range_excerpt_without_line_prefixes(text));
                    if let Ok(nested) = serde_json::from_str::<serde_json::Value>(text) {
                        collect_machine_text_fragments_from_json(&nested, values);
                    }
                }
            }
            for key in ["extra", "result", "data"] {
                if let Some(child) = obj.get(key) {
                    collect_machine_text_fragments_from_json(child, values);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_machine_text_fragments_from_json(item, values);
            }
        }
        serde_json::Value::String(text) => {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                values.push(trimmed.to_string());
                if let Ok(nested) = serde_json::from_str::<serde_json::Value>(trimmed) {
                    collect_machine_text_fragments_from_json(&nested, values);
                }
            }
        }
        serde_json::Value::Number(value) => values.push(value.to_string()),
        serde_json::Value::Bool(value) => values.push(value.to_string()),
        serde_json::Value::Null => {}
    }
}

fn machine_kv_pairs_grounded_by_observation(
    pairs: &[(String, String)],
    observed_texts: &[String],
) -> bool {
    pairs
        .iter()
        .filter(|(_, value)| !machine_value_is_inline_literal(value))
        .all(|pair| machine_kv_pair_has_observed_value(pair, observed_texts))
}

fn machine_kv_pair_has_observed_value(pair: &(String, String), observed_texts: &[String]) -> bool {
    observed_texts
        .iter()
        .any(|text| text.contains(pair.1.as_str()))
}

fn machine_marker_is_observed(marker: &str, observed_texts: &[String]) -> bool {
    observed_texts.iter().any(|text| text.contains(marker))
}

fn machine_value_is_inline_literal(value: &str) -> bool {
    if matches!(
        value,
        "yes" | "no" | "true" | "false" | "ok" | "required" | "forbidden" | "enabled"
    ) {
        return true;
    }
    if value.chars().all(|ch| ch.is_ascii_digit()) {
        return true;
    }
    if value.split(',').count() > 1 {
        return value.split(',').all(machine_value_is_short_machine_atom);
    }
    machine_value_is_short_machine_atom(value)
        && !value.chars().any(|ch| matches!(ch, '.' | '/' | ':' | '@'))
}

fn machine_value_is_short_machine_atom(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
}

fn read_range_excerpt_without_line_prefixes(excerpt: &str) -> String {
    excerpt
        .lines()
        .map(strip_read_range_line_prefix)
        .collect::<Vec<_>>()
        .join("\n")
}

fn strip_read_range_line_prefix(line: &str) -> &str {
    let Some((prefix, rest)) = line.split_once('|') else {
        return line;
    };
    let prefix = prefix.trim();
    if prefix.is_empty() || prefix.len() > 6 || !prefix.chars().all(|ch| ch.is_ascii_digit()) {
        return line;
    }
    rest
}

#[cfg(test)]
#[path = "machine_kv_projection_tests.rs"]
mod tests;

pub(crate) fn requested_machine_kv_summary_from_observations(
    input: &str,
    observed_texts: &[String],
) -> Option<String> {
    let template_markers = requested_machine_value_template_markers(input)
        .into_iter()
        .filter(|marker| !machine_request_option_key(marker))
        .collect::<Vec<_>>();
    let pairs = requested_machine_kv_pairs(input)
        .into_iter()
        .filter(|(key, _)| !machine_request_option_key(key))
        .collect::<Vec<_>>();
    let pair_keys = pairs
        .iter()
        .map(|(key, _)| key.as_str())
        .collect::<Vec<_>>();
    let mut markers = requested_machine_markers(input)
        .into_iter()
        .filter(|marker| !pair_keys.iter().any(|key| key == marker))
        .filter(|marker| !template_markers.iter().any(|key| key == marker))
        .filter(|marker| !machine_request_option_key(marker))
        .filter(|marker| !observed_only_as_machine_identity_value(marker, observed_texts))
        .collect::<Vec<_>>();
    if !template_markers.is_empty() {
        markers.retain(|marker| {
            observed_machine_marker_projection(marker, observed_texts).is_some()
                || !valid_single_machine_marker(marker)
        });
    }
    if (pairs.is_empty() && markers.is_empty() && template_markers.is_empty())
        || observed_texts.is_empty()
    {
        return None;
    }
    if !markers
        .iter()
        .all(|marker| machine_marker_is_observed(marker, observed_texts))
    {
        return None;
    }
    if !template_markers
        .iter()
        .all(|marker| observed_value_projection_for_template(marker, observed_texts).is_some())
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
    let mut parts = Vec::new();
    for marker in markers {
        if let Some(projected) = observed_machine_marker_projection(marker.as_str(), observed_texts)
        {
            parts.push(projected);
        } else if valid_single_machine_marker(marker.as_str()) {
            return None;
        } else {
            parts.push(marker);
        }
    }
    for marker in template_markers {
        let value = observed_value_projection_for_template(marker.as_str(), observed_texts)?;
        parts.push(format!("{marker}={value}"));
    }
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

fn requested_machine_value_template_markers(input: &str) -> Vec<String> {
    let mut markers = Vec::new();
    for token in input.split_whitespace().map(trim_machine_kv_token) {
        push_machine_value_template_marker(token, &mut markers);
    }
    for segment in input.split(machine_token_boundary) {
        let token = trim_machine_kv_token(segment);
        push_machine_value_template_marker(token, &mut markers);
    }
    markers
}

fn push_machine_value_template_marker(token: &str, markers: &mut Vec<String>) {
    let Some((key, value)) = token.split_once('=') else {
        return;
    };
    let key = key.trim();
    let value = value.trim();
    if valid_machine_key(key)
        && !filename_like_machine_token(key)
        && machine_value_template_placeholder(value)
        && !markers.iter().any(|existing| existing == key)
    {
        markers.push(key.to_string());
    }
}

fn machine_value_template_placeholder(value: &str) -> bool {
    let normalized = value
        .trim()
        .trim_matches(|ch| matches!(ch, '<' | '>' | '{' | '}'))
        .trim()
        .to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "value" | "field_value" | "result" | "answer"
    )
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
    if filename_like_machine_token(value) {
        return false;
    }
    if valid_machine_key(value) && valid_single_machine_marker(value) {
        return true;
    }
    valid_machine_key(value)
        && (value.contains('.') || value.contains('_'))
        && value
            .split('.')
            .all(|part| !part.is_empty() && part.chars().any(|ch| ch.is_ascii_alphabetic()))
}

fn filename_like_machine_token(value: &str) -> bool {
    let parts = value.split('.').collect::<Vec<_>>();
    let [stem, extension] = parts.as_slice() else {
        return false;
    };
    if stem.is_empty()
        || extension.is_empty()
        || !stem
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
    {
        return false;
    }
    matches!(
        extension.to_ascii_lowercase().as_str(),
        "bak"
            | "conf"
            | "css"
            | "csv"
            | "db"
            | "env"
            | "html"
            | "ini"
            | "js"
            | "json"
            | "jsonl"
            | "lock"
            | "log"
            | "md"
            | "py"
            | "rs"
            | "sh"
            | "sql"
            | "sqlite"
            | "sqlite3"
            | "toml"
            | "ts"
            | "tsx"
            | "txt"
            | "yaml"
            | "yml"
    )
}

fn valid_single_machine_marker(value: &str) -> bool {
    matches!(
        value,
        "path"
            | "exists"
            | "count"
            | "status"
            | "state"
            | "states"
            | "target"
            | "task_id"
            | "member"
            | "member_path"
            | "member_count"
            | "checkpoint_id"
            | "checkpoint_id_present"
            | "can_poll"
            | "can_cancel"
            | "rows"
            | "tables"
            | "table_count"
            | "members"
            | "content_excerpt"
            | "candidate_count"
            | "planned_groups"
            | "would_move"
            | "photo_count"
            | "names"
            | "manager"
            | "available"
            | "valid"
            | "branch"
            | "hash"
            | "port"
            | "title"
            | "timezone"
            | "datetime"
            | "source"
            | "price"
            | "symbol"
            | "code"
            | "name"
            | "location"
            | "temperature"
            | "weather_code"
            | "weather_code_raw"
            | "city"
            | "query"
            | "provider"
            | "model"
    )
}

fn machine_request_option_key(value: &str) -> bool {
    matches!(
        value,
        "names_only"
            | "max_entries"
            | "include_hidden"
            | "files_only"
            | "dirs_only"
            | "directories_only"
            | "start_line"
            | "end_line"
            | "slice_mode"
            | "slice_n"
            | "slice_start"
            | "slice_end"
            | "max_bytes"
            | "max_results"
            | "limit"
            | "sort_by"
            | "output_format"
            | "stat_paths"
            | "list_dir"
            | "count_entries"
            | "read_range"
            | "read_text_range"
            | "find_entries"
            | "grep_text"
            | "compare_paths"
            | "tree_summary"
            | "workspace_glance"
            | "path_batch_facts"
            | "extract_field"
            | "extract_fields"
            | "structured_keys"
            | "validate_structured"
    )
}

fn observed_only_as_machine_identity_value(marker: &str, observed_texts: &[String]) -> bool {
    if observed_machine_marker_projection(marker, observed_texts).is_some() {
        return false;
    }
    let identity_prefixes = [
        "action=",
        "extra.action=",
        "kind=",
        "extra.kind=",
        "mode=",
        "extra.mode=",
        "recommended_mode=",
        "extra.recommended_mode=",
        "type=",
        "extra.type=",
        "skill=",
        "extra.skill=",
        "resolved_tool_or_skill=",
        "requested_action_ref=",
        "requested_capability=",
    ];
    observed_texts.iter().any(|text| {
        identity_prefixes.iter().any(|prefix| {
            text.strip_prefix(prefix)
                .is_some_and(|value| value == marker)
        })
    })
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
        push_machine_text_fragment_with_lines(trimmed, values);
    }
}

fn push_machine_text_fragment_with_lines(text: &str, values: &mut Vec<String>) {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return;
    }
    values.push(trimmed.to_string());
    for line in trimmed
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        if line != trimmed {
            values.push(line.to_string());
            if let Some(unbulleted) = strip_machine_list_marker(line) {
                values.push(unbulleted.to_string());
            }
        }
    }
}

fn strip_machine_list_marker(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    for prefix in ["- ", "* ", "+ "] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let rest = rest.trim_start();
            if !rest.is_empty() {
                return Some(rest);
            }
        }
    }
    let (head, rest) = trimmed.split_once('.')?;
    if head.is_empty() || head.len() > 3 || !head.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    let rest = rest.trim_start();
    (!rest.is_empty()).then_some(rest)
}

fn collect_machine_text_fragments_from_json(value: &serde_json::Value, values: &mut Vec<String>) {
    collect_machine_text_fragments_from_json_path(value, None, values);
}

fn collect_machine_text_fragments_from_json_path(
    value: &serde_json::Value,
    parent_path: Option<&str>,
    values: &mut Vec<String>,
) {
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
                    collect_machine_text_fragments_from_json_path(child, Some(key), values);
                }
            }
            for (key, child) in obj {
                if valid_machine_key(key) {
                    values.push(key.to_string());
                    if let Some(parent) = parent_path {
                        values.push(format!("{parent}.{key}"));
                    }
                    if let Some(value) = machine_json_value_as_surface_for_key(key, child) {
                        values.push(format!("{key}={value}"));
                        if let Some(parent) = parent_path {
                            values.push(format!("{parent}.{key}={value}"));
                        }
                    } else if let Some(value) = machine_scalar_json_value_as_surface(child) {
                        values.push(format!("{key}={value}"));
                        if let Some(parent) = parent_path {
                            values.push(format!("{parent}.{key}={value}"));
                        }
                    } else if let Some(value) = machine_array_json_value_as_surface(child) {
                        values.push(format!("{key}={value}"));
                        if let Some(parent) = parent_path {
                            values.push(format!("{parent}.{key}={value}"));
                        }
                    }
                }
                collect_machine_text_fragments_from_json_path(child, Some(key), values);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_machine_text_fragments_from_json_path(item, parent_path, values);
            }
        }
        serde_json::Value::String(text) => {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                push_machine_text_fragment_with_lines(trimmed, values);
                if let Ok(nested) = serde_json::from_str::<serde_json::Value>(trimmed) {
                    collect_machine_text_fragments_from_json_path(&nested, parent_path, values);
                }
            }
        }
        serde_json::Value::Number(value) => values.push(value.to_string()),
        serde_json::Value::Bool(value) => values.push(value.to_string()),
        serde_json::Value::Null => {}
    }
}

fn machine_json_value_as_surface_for_key(key: &str, value: &serde_json::Value) -> Option<String> {
    if !matches!(key, "content_excerpt") {
        return None;
    }
    let text = value.as_str()?.trim();
    if text.is_empty() || text.len() > 240 || text.contains(|ch| matches!(ch, '\0' | '\r' | '\n')) {
        return None;
    }
    if valid_machine_scalar_value_atom(text) {
        return Some(text.to_string());
    }
    serde_json::to_string(text).ok()
}

fn machine_scalar_json_value_as_surface(value: &serde_json::Value) -> Option<String> {
    let surface = match value {
        serde_json::Value::String(text) => text.trim().to_string(),
        serde_json::Value::Number(number) => number.to_string(),
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) | serde_json::Value::Null => {
            return None;
        }
    };
    valid_machine_scalar_value_atom(surface.as_str()).then_some(surface)
}

fn machine_array_json_value_as_surface(value: &serde_json::Value) -> Option<String> {
    let serde_json::Value::Array(items) = value else {
        return None;
    };
    if items.is_empty() || items.len() > 16 {
        return None;
    }
    let mut values = Vec::new();
    for item in items {
        values.push(machine_scalar_json_value_as_surface(item)?);
    }
    serde_json::to_string(&values).ok()
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

fn observed_machine_marker_projection(marker: &str, observed_texts: &[String]) -> Option<String> {
    let prefix = format!("{marker}=");
    observed_texts
        .iter()
        .find_map(|text| {
            text.strip_prefix(prefix.as_str())
                .and_then(projected_machine_value)
        })
        .map(|value| format!("{marker}={value}"))
}

fn projected_machine_value(raw: &str) -> Option<String> {
    let value = raw.trim();
    if value.is_empty() {
        return None;
    }
    if matches!(value.as_bytes().first(), Some(b'[' | b'{')) {
        if let Some(json_value) = first_json_value_prefix(value) {
            return serde_json::to_string(&json_value).ok();
        }
    }
    let first_line = value.lines().next()?.trim();
    let end = inline_machine_pair_boundary(first_line).unwrap_or(first_line.len());
    let projected = first_line[..end]
        .trim()
        .trim_end_matches(|ch| matches!(ch, ',' | ';'));
    (!projected.is_empty()).then(|| projected.to_string())
}

fn first_json_value_prefix(value: &str) -> Option<serde_json::Value> {
    let mut stream = serde_json::Deserializer::from_str(value).into_iter::<serde_json::Value>();
    stream.next()?.ok()
}

fn inline_machine_pair_boundary(line: &str) -> Option<usize> {
    for (idx, ch) in line.char_indices().skip(1) {
        if !matches!(ch, ' ' | ',') {
            continue;
        }
        let after = line[idx + ch.len_utf8()..].trim_start();
        let Some((key, value)) = after.split_once('=') else {
            continue;
        };
        if valid_machine_key(key.trim()) && !value.trim().is_empty() {
            return Some(idx);
        }
    }
    None
}

fn observed_value_projection_for_template(
    marker: &str,
    observed_texts: &[String],
) -> Option<String> {
    if let Some(projected) = observed_machine_marker_projection(marker, observed_texts) {
        return projected
            .split_once('=')
            .map(|(_, value)| value.trim().to_string())
            .filter(|value| valid_machine_scalar_value_atom(value));
    }
    let prefixes = [
        "extra.value_text=",
        "value_text=",
        "extra.field_value=",
        "field_value=",
        "extra.value=",
        "value=",
    ];
    let mut candidates = Vec::new();
    for prefix in prefixes {
        for value in observed_texts
            .iter()
            .filter_map(|text| text.strip_prefix(prefix).map(str::trim))
            .filter(|value| valid_machine_scalar_value_atom(value))
        {
            if !candidates.iter().any(|existing| existing == value) {
                candidates.push(value.to_string());
            }
        }
    }
    match candidates.as_slice() {
        [value] => Some(value.clone()),
        _ => None,
    }
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

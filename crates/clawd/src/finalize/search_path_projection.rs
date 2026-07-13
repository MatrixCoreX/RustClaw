use serde_json::Value;

pub(crate) fn path_listing_from_marker_summary_outputs<'a>(
    outputs: impl IntoIterator<Item = &'a str>,
    requested_summary: &str,
) -> Option<String> {
    let marker = marker_only_summary(requested_summary)?;
    let mut filtered_paths = Vec::new();
    let mut fallback_paths = Vec::new();
    for output in outputs {
        let Ok(value) = serde_json::from_str::<Value>(output.trim()) else {
            continue;
        };
        collect_search_paths_for_marker(&value, marker, &mut filtered_paths, &mut fallback_paths);
    }
    if !filtered_paths.is_empty() {
        return Some(filtered_paths.join("\n"));
    }
    if !fallback_paths.is_empty() {
        return Some(fallback_paths.join("\n"));
    }
    None
}

fn marker_only_summary(summary: &str) -> Option<&str> {
    let marker = summary.trim();
    if marker.is_empty()
        || marker.contains('=')
        || marker
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count()
            != 1
        || !valid_machine_marker(marker)
    {
        return None;
    }
    Some(marker)
}

fn valid_machine_marker(marker: &str) -> bool {
    marker
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
}

fn collect_search_paths_for_marker(
    value: &Value,
    marker: &str,
    filtered_paths: &mut Vec<String>,
    fallback_paths: &mut Vec<String>,
) {
    if let Some(extra) = value.get("extra") {
        collect_search_paths_for_marker(extra, marker, filtered_paths, fallback_paths);
    }
    if let Some(result) = value.get("result") {
        collect_search_paths_for_marker(result, marker, filtered_paths, fallback_paths);
    }
    let Some(object) = value.as_object() else {
        return;
    };
    let action = object.get("action").and_then(Value::as_str).unwrap_or("");
    if !matches!(action, "find_entries" | "grep_text") || !search_payload_refs_marker(value, marker)
    {
        return;
    }
    collect_path_values_at_keys(object, &["name_results", "results"], filtered_paths);
    collect_match_paths(object.get("matches"), fallback_paths);
}

fn search_payload_refs_marker(value: &Value, marker: &str) -> bool {
    value_contains_marker_at_keys(value, marker, &["query", "name_pattern", "pattern"])
        || array_contains_marker_at_keys(value, marker, &["patterns", "name_patterns"])
        || any_path_contains_marker(value, marker)
}

fn value_contains_marker_at_keys(value: &Value, marker: &str, keys: &[&str]) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    keys.iter().any(|key| {
        object
            .get(*key)
            .and_then(Value::as_str)
            .is_some_and(|text| text.contains(marker))
    })
}

fn array_contains_marker_at_keys(value: &Value, marker: &str, keys: &[&str]) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    keys.iter().any(|key| {
        object
            .get(*key)
            .and_then(Value::as_array)
            .is_some_and(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .any(|text| text.contains(marker))
            })
    })
}

fn any_path_contains_marker(value: &Value, marker: &str) -> bool {
    let mut paths = Vec::new();
    collect_path_values(value, &mut paths);
    paths.iter().any(|path| path.contains(marker))
}

fn collect_path_values_at_keys(
    object: &serde_json::Map<String, Value>,
    keys: &[&str],
    paths: &mut Vec<String>,
) {
    for key in keys {
        if let Some(value) = object.get(*key) {
            collect_path_values(value, paths);
        }
    }
}

fn collect_match_paths(value: Option<&Value>, paths: &mut Vec<String>) {
    let Some(Value::Array(items)) = value else {
        return;
    };
    for item in items {
        if let Some(path) = item.get("path").and_then(Value::as_str) {
            push_unique_path(paths, path);
        }
    }
}

fn collect_path_values(value: &Value, paths: &mut Vec<String>) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_path_values(item, paths);
            }
        }
        Value::Object(object) => {
            if let Some(path) = object
                .get("path")
                .or_else(|| object.get("resolved_path"))
                .and_then(Value::as_str)
            {
                push_unique_path(paths, path);
            }
            for key in ["results", "name_results", "matches"] {
                if let Some(value) = object.get(key) {
                    collect_path_values(value, paths);
                }
            }
        }
        Value::String(path) => push_unique_path(paths, path),
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn push_unique_path(paths: &mut Vec<String>, path: &str) {
    let path = path.trim();
    if !path_like(path) || paths.iter().any(|existing| existing == path) {
        return;
    }
    paths.push(path.to_string());
}

fn path_like(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 512
        && !path.contains(|ch: char| ch.is_control() || ch.is_whitespace())
        && (path.contains('/') || path.starts_with('.') || path.rsplit_once('.').is_some())
}
